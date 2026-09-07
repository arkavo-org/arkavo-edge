//! Codex worker wiring shared by every Arkavo entry point.
//!
//! Four places need a Codex worker: the CLI tool loop, `LocalEngine`, the A2A
//! server, and the `arkavo codex` command. Each one would otherwise carry its
//! own copy of the rate card, the admission estimate and the session-state
//! rule, and a second copy of a price is a second price. This module owns all
//! three so the ledger, the admission gate and the state layout are identical
//! wherever a worker is opened.
//!
//! Prices are read out of the router's table rather than restated here: the
//! router is already the single source of truth for what an OpenAI model costs.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use anyhow::{Context, Result, ensure};
use arkavo_budget::{BudgetTracker, CloudPolicy, PricingEntry, TokenCost, cost::TokenUsage};
use arkavo_mcp_codex::{CodexConfig, CodexWorker, SpendApproval};
use arkavo_mcp_tools::ToolRegistry;
use arkavo_router::{ModelChoice, TaskCategory};
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

/// Codex runs this model; the router prices it. `CodexConfig::discover` picks
/// the same id, and the crate refuses a `SpendApproval` whose pricing names a
/// different model or provider.
const MODEL: ModelChoice = ModelChoice::Gpt6Astra;

/// Probe size used to read a per-MTok rate back out of the router's table.
/// `usage_cost_usd` is linear in each usage dimension, so any probe recovers
/// the rate — but a probe at or above Astra's 272k long-context boundary would
/// return the doubled tier as though it were the base rate.
const PROBE_TOKENS: u32 = 1_000;

/// Astra's long-context tier starts here; a probe at or above it would read the
/// doubled rate back as the base rate.
const LONG_CONTEXT_TOKENS: u32 = 272_000;
const _: () = assert!(PROBE_TOKENS < LONG_CONTEXT_TOKENS);

/// Recover one dimension's cents-per-MTok from the router's dollar pricing.
fn cents_per_mtok(usage: &TokenUsage) -> u64 {
    let per_probe_dollars = MODEL.usage_cost_usd(usage);
    let per_mtok_dollars = per_probe_dollars * (1_000_000.0 / f64::from(PROBE_TOKENS));
    (per_mtok_dollars * 100.0).round() as u64
}

/// The Codex worker's rate card, derived from the router's price table.
fn pricing() -> PricingEntry {
    // Every rate is a total for its own bucket, matching both the router's
    // `calculate_cost` and `CodexWorker::cost`, which makes the buckets
    // disjoint before pricing them. Nothing here is a surcharge over another
    // rate, so each dimension is probed and passed through unchanged.
    PricingEntry {
        model_id: MODEL.name().to_string(),
        provider: MODEL.provider().to_string(),
        input_cents_per_mtok: cents_per_mtok(&TokenUsage {
            input_tokens: PROBE_TOKENS,
            ..Default::default()
        }),
        output_cents_per_mtok: cents_per_mtok(&TokenUsage {
            output_tokens: PROBE_TOKENS,
            ..Default::default()
        }),
        cached_input_cents_per_mtok: Some(cents_per_mtok(&TokenUsage {
            cached_input_tokens: PROBE_TOKENS,
            ..Default::default()
        })),
        cache_write_cents_per_mtok: Some(cents_per_mtok(&TokenUsage {
            cache_write_tokens: PROBE_TOKENS,
            ..Default::default()
        })),
        context_window: None,
        max_output_tokens: None,
    }
}

/// Admission estimate for one delegated Codex run.
///
/// This is the router's own per-request estimate for a code-generation task on
/// this model, priced through the same table as the ledger. A Codex `exec` run
/// is an internal agentic loop rather than the single turn that estimate
/// covers, so a long run routinely exceeds it — which is why the worker treats
/// the figure as admission only and records measured usage afterwards. What the
/// gate guarantees is that a run cannot start against an exhausted cap, not
/// that it will stay inside one.
fn admission_estimate() -> TokenCost {
    let tokens = TaskCategory::CodeGeneration.estimated_tokens();
    TokenCost::from_dollars(MODEL.usage_cost_usd(&TokenUsage {
        input_tokens: tokens.input,
        output_tokens: tokens.output,
        ..Default::default()
    }))
}

/// Host spend authority for a Codex worker.
///
/// `user_confirmed` is the host's assertion that a person asked for *this* run;
/// it is consumed by one attempt and can never be set from tool arguments.
pub fn spend_approval(policy: CloudPolicy, user_confirmed: bool) -> SpendApproval {
    SpendApproval {
        policy,
        user_confirmed,
        projected_cost: admission_estimate(),
        pricing: pricing(),
    }
}

/// Session-state file name for one agent identity in one workspace.
///
/// The name is stable across restarts on purpose: a crash mid-run leaves the
/// binding marked `accounting_incomplete`, and only a file the next process
/// reopens can force the host to reconcile that charge. The workspace digest
/// keeps one identity's sessions in different checkouts apart — the store
/// rejects a saved binding whose workspace changed, so a shared name would
/// make the worker unusable outside the directory it first ran in.
fn state_file_name(agent_id: &str, workspace: &Path) -> Result<String> {
    ensure!(
        !agent_id.is_empty()
            && agent_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "Agent identity must be an ASCII alphanumeric, '-' or '_' file name component"
    );
    let digest = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
    Ok(format!("{agent_id}-{}.json", &digest[..16]))
}

/// Host-owned session state, outside the worker's workspace as the crate requires.
///
/// Public so the `arkavo codex` command names the same file this module does:
/// one identity in one workspace has one session, however it was started.
pub fn state_path(agent_id: &str, workspace: &Path) -> Result<PathBuf> {
    let root = std::env::home_dir()
        .context("Cannot determine home directory for Codex session state")?
        .join(".arkavo")
        .join("codex");
    std::fs::create_dir_all(&root)?;
    Ok(root.join(state_file_name(agent_id, workspace)?))
}

/// Workers keyed by the identity and workspace their session is bound to,
/// carrying the spend policy each one was opened with.
type WorkerCache = HashMap<(String, PathBuf), (CloudPolicy, Arc<CodexWorker>)>;

/// One worker per identity, workspace and spend policy, for the life of the
/// process.
///
/// The session store takes an exclusive lock on its state file, so a second
/// `CodexWorker::open` for the same session fails while the first is alive —
/// and registries are rebuilt while the previous one is still referenced (a new
/// CLI request, an agent-config hot-reload on the A2A server). Opening per
/// registration would therefore make the tools vanish after the first rebuild.
/// Caching keeps one session, and one lock, per process.
///
/// A worker carries the spend policy it was opened with, so the cached entry is
/// only reusable while the requested policy still matches. When AGENTS.md
/// changes it, the entry is dropped and the session reopened: the file lock is
/// the arbiter. If anything still holds the old worker the reopen fails, the
/// tools are not registered, and no run happens under the superseded policy —
/// the safe outcome for a tightening, and a recoverable one because the cache
/// no longer hides a stale worker behind the key.
static WORKERS: LazyLock<Mutex<WorkerCache>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// Open, or reuse, the worker for this config's identity and workspace.
///
/// The caller owns the sandbox decision; `CodexConfig::discover` defaults to
/// read-only and only an explicit host grant widens it.
fn shared_worker(
    config: CodexConfig,
    policy: CloudPolicy,
    budget: Arc<BudgetTracker>,
) -> Result<Arc<CodexWorker>> {
    let key = (config.agent_id.clone(), config.workspace.clone());
    let mut workers = WORKERS
        .lock()
        .map_err(|_| anyhow::anyhow!("Codex worker cache lock poisoned"))?;
    match workers.get(&key) {
        Some((cached, worker)) if *cached == policy => return Ok(worker.clone()),
        // Dropping the entry releases this reference to the session; the store
        // lock is then free unless a live registry or an in-flight run holds it.
        Some(_) => {
            workers.remove(&key);
        }
        None => {}
    }
    let state = state_path(&config.agent_id, &config.workspace)?;
    let worker = Arc::new(CodexWorker::open(
        config,
        &state,
        spend_approval(policy, false),
        budget,
    )?);
    workers.insert(key, (policy, worker.clone()));
    Ok(worker)
}

/// Register `codex_run`, `codex_status` and `codex_cancel` when a Codex CLI is
/// on PATH and the spend policy could ever authorize a run.
///
/// Registered workers get the read-only sandbox: an LLM-reachable tool must not
/// carry a workspace-write grant that no person made for that run. Writes stay
/// with the `arkavo codex --write` command, where a human issues the grant.
///
/// `user_confirmed` is false here — a tool call is not a person. Under
/// `AskBeforeCloud` the tools are still registered and `codex_run` refuses at
/// call time with the spend plane's `NeedsUserConfirmation` verdict, which is
/// how every other cloud path behaves under that policy.
pub fn register_tools(
    registry: &mut ToolRegistry,
    budget: Arc<BudgetTracker>,
    policy: CloudPolicy,
    agent_id: &str,
) {
    if policy == CloudPolicy::LocalOnly {
        debug!("Codex tools skipped: cloud policy is local-only");
        return;
    }
    let workspace = match std::env::current_dir() {
        Ok(workspace) => workspace,
        Err(e) => {
            warn!("Codex tools disabled: no current directory: {e}");
            return;
        }
    };
    let config = match CodexConfig::discover(workspace, agent_id.to_string()) {
        Ok(config) => config,
        Err(e) => {
            debug!("Codex tools skipped: {e}");
            return;
        }
    };
    match shared_worker(config, policy, budget) {
        Ok(worker) => {
            arkavo_mcp_codex::register_tools(registry, worker);
            info!("Codex MCP tools registered for {agent_id} (read-only sandbox)");
        }
        Err(e) => warn!("Codex tools disabled: {e}"),
    }
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "Tokio test macros create a runtime from a synchronous test entry point"
)]
mod tests {
    use super::*;

    #[test]
    fn rate_card_is_the_router_base_tier() {
        let pricing = pricing();
        assert_eq!(pricing.model_id, "gpt-6-astra");
        assert_eq!(pricing.provider, "openai");
        // $10 / $50 / $1 / $12.50 per MTok, in cents.
        assert_eq!(pricing.input_cents_per_mtok, 1_000);
        assert_eq!(pricing.output_cents_per_mtok, 5_000);
        assert_eq!(pricing.cached_input_cents_per_mtok, Some(100));
        assert_eq!(pricing.cache_write_cents_per_mtok, Some(1_250));
    }

    /// Every entry is the router's total for its own bucket. `CodexWorker::cost`
    /// makes the buckets disjoint before pricing them, exactly as the router's
    /// `calculate_cost` does, so a rate that were a surcharge over another —
    /// or a bucket left unpriced — would mis-bill every cached-prompt run.
    #[test]
    fn every_rate_is_the_router_total_for_its_own_bucket() {
        let pricing = pricing();
        for (rate, tokens) in [
            (
                pricing.input_cents_per_mtok,
                TokenUsage {
                    input_tokens: PROBE_TOKENS,
                    ..Default::default()
                },
            ),
            (
                pricing.output_cents_per_mtok,
                TokenUsage {
                    output_tokens: PROBE_TOKENS,
                    ..Default::default()
                },
            ),
            (
                pricing.cached_input_cents_per_mtok.expect("cached rate"),
                TokenUsage {
                    cached_input_tokens: PROBE_TOKENS,
                    ..Default::default()
                },
            ),
            (
                pricing
                    .cache_write_cents_per_mtok
                    .expect("cache-write rate"),
                TokenUsage {
                    cache_write_tokens: PROBE_TOKENS,
                    ..Default::default()
                },
            ),
        ] {
            assert_eq!(rate, cents_per_mtok(&tokens));
        }
    }

    #[test]
    fn admission_estimate_is_positive_and_router_priced() {
        // CodeGeneration is 800 input + 3000 output tokens: $0.008 + $0.15.
        assert_eq!(admission_estimate().as_cents(), 15);
        assert!(!admission_estimate().is_zero());
    }

    #[test]
    fn approval_matches_the_model_the_crate_validates_against() {
        let approval = spend_approval(CloudPolicy::CloudWithinCap, false);
        assert_eq!(approval.pricing.model_id, MODEL.name());
        assert!(!approval.user_confirmed);
        assert_eq!(approval.projected_cost, admission_estimate());
    }

    #[test]
    fn state_name_is_stable_per_workspace_and_distinct_across_them() {
        let one = state_file_name("arkavo-cli", Path::new("/tmp/one")).expect("name");
        let again = state_file_name("arkavo-cli", Path::new("/tmp/one")).expect("name");
        let other = state_file_name("arkavo-cli", Path::new("/tmp/two")).expect("name");
        assert_eq!(one, again);
        assert_ne!(one, other);
        assert!(one.starts_with("arkavo-cli-") && one.ends_with(".json"));
    }

    /// A session in a throwaway workspace, under an identity no other test
    /// shares, so the process-global cache cannot couple two tests together.
    struct Session {
        _workspace: tempfile::TempDir,
        root: PathBuf,
        agent_id: String,
        state: PathBuf,
        budget: Arc<BudgetTracker>,
    }

    impl Session {
        async fn new() -> Self {
            let workspace = tempfile::tempdir().expect("workspace");
            // The worker canonicalizes its workspace and the state name is a
            // digest of that path, so the test must name the file it will.
            let root = workspace.path().canonicalize().expect("workspace");
            let agent_id = format!("test-{}", uuid::Uuid::new_v4().simple());
            let state = state_path(&agent_id, &root).expect("state path");
            let budget = Arc::new(
                arkavo_budget::BudgetTracker::new(arkavo_budget::BudgetConfig::default())
                    .await
                    .expect("budget"),
            );
            Self {
                _workspace: workspace,
                root,
                agent_id,
                state,
                budget,
            }
        }

        fn config(&self) -> CodexConfig {
            CodexConfig {
                executable: std::env::current_exe().expect("test executable"),
                workspace: self.root.clone(),
                agent_id: self.agent_id.clone(),
                model: MODEL.name().to_string(),
                sandbox: arkavo_mcp_codex::Sandbox::ReadOnly,
                timeout: std::time::Duration::from_secs(1),
                max_output_bytes: 1024,
            }
        }

        fn open(&self, policy: CloudPolicy) -> Result<Arc<CodexWorker>> {
            shared_worker(self.config(), policy, self.budget.clone())
        }

        fn key(&self) -> (String, PathBuf) {
            (self.agent_id.clone(), self.root.clone())
        }

        fn cached_policy(&self) -> Option<CloudPolicy> {
            WORKERS
                .lock()
                .expect("cache")
                .get(&self.key())
                .map(|(policy, _)| *policy)
        }
    }

    impl Drop for Session {
        fn drop(&mut self) {
            WORKERS.lock().expect("cache").remove(&self.key());
            std::fs::remove_file(&self.state).ok();
            std::fs::remove_file(self.state.with_extension("lock")).ok();
        }
    }

    /// A registry rebuild — a new CLI request, an agent-config hot-reload —
    /// re-registers while the previous registry still holds the worker, and the
    /// session store's lock is exclusive. Opening a second worker would fail and
    /// the tools would silently disappear, so the same worker must come back.
    #[tokio::test]
    async fn the_same_session_is_reused_across_registry_rebuilds() {
        let session = Session::new().await;
        let first = session
            .open(CloudPolicy::CloudWithinCap)
            .expect("first open");
        let second = session
            .open(CloudPolicy::CloudWithinCap)
            .expect("reopen must reuse the locked session");
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(session.cached_policy(), Some(CloudPolicy::CloudWithinCap));
    }

    /// An AGENTS.md reload that tightens the policy must not be answered from
    /// the cache. While the looser worker is still referenced the session lock
    /// refuses the reopen, so registration is skipped rather than served a
    /// worker admitted under the superseded posture; once that reference is
    /// gone the next request opens a worker carrying the new policy.
    #[tokio::test]
    async fn a_tightened_policy_is_never_served_from_the_cache() {
        let session = Session::new().await;
        let loose = session
            .open(CloudPolicy::CloudWithinCap)
            .expect("first open");

        assert!(session.open(CloudPolicy::AskBeforeCloud).is_err());
        // The stale entry is gone, so the tightening is not hidden behind the
        // key until the process restarts.
        assert_eq!(session.cached_policy(), None);

        drop(loose);
        let tight = session
            .open(CloudPolicy::AskBeforeCloud)
            .expect("released session reopens under the new policy");
        assert_eq!(session.cached_policy(), Some(CloudPolicy::AskBeforeCloud));
        drop(tight);
    }

    #[test]
    fn state_name_rejects_identities_that_would_escape_the_directory() {
        for id in ["", "../escape", "with/slash", "with space", "dot.dot"] {
            assert!(
                state_file_name(id, Path::new("/tmp")).is_err(),
                "{id} was accepted as a file name component"
            );
        }
    }
}
