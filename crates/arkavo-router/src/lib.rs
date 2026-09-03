#![allow(clippy::significant_drop_tightening)]
// Rust 1.98 flags the same shape under a second name for functions in impl blocks.
#![allow(clippy::unused_async, clippy::unused_async_trait_impl)]

pub mod architect;
pub mod classifier;
pub mod connectivity;
pub mod decision;
pub mod deliberation;
pub mod error;
pub mod health;
pub mod judge;
pub mod learning;
pub mod metrics;
pub mod model_discovery;
pub mod optimal_config;
pub mod orchestrator;
pub mod planes;
pub mod prediction;
pub mod preflight;
pub mod prompt_advisor;
pub mod provider;
pub mod provider_info;
#[cfg(feature = "llama-cpp")]
pub(crate) mod provider_protected;
pub(crate) mod quality_gate;
pub mod response;
pub mod rlm;
pub mod selector;
pub mod selector_quality;
pub mod spec_stats;
pub mod stream;
#[cfg(feature = "tdf-encrypt")]
pub mod tdf_audit;
pub mod tool_extraction;
pub mod tool_request_parser;
pub mod tools;
pub mod validator;

pub use architect::{
    ArchitectExecutor, ArchitectPlan, ArchitectPlanner, ArchitectResult, ComplexityScore,
    ComplexityScorer, Subtask, SubtaskResult,
};
pub use classifier::{TaskCategory, TaskClassifier, classify_task_keywords};
pub use connectivity::ConnectivityChecker;
pub use decision::{ModelChoice, PlannerTier, RoutingDecision};
pub use deliberation::{DeliberationConfig, DeliberationResult, Deliberator};
pub use error::{Error, Result};
pub use judge::{IssueType, JudgmentResult, ResponseJudge};
pub use metrics::RoutingMetrics;
pub use orchestrator::{
    ArchitectRoutingResult, CostOrchestrator, CostRecommendation, OrchestratorMetrics,
    ScalingDecision,
};
pub use planes::{
    AnswerObservation, CollapseSignal, CollapseVerdict, FeasibilityBaseline,
    FeasibilityBaselineSnapshot, FeasibilityVerdict, LocalPlan, RuntimeStats, UpgradeContext,
    UpgradeOffer, assess_feasibility, augment_exclusions_for_policy, authorize_upgrade,
    detect_collapse, plan_local, upgrade_offer,
};
pub use prediction::{BudgetRunway, WorkflowCostPrediction, WorkflowCostPredictor};
pub use preflight::{
    AgentConfig, BudgetYamlConfig, KasTrustedRootYaml, KasYamlConfig, ModerationResult, PolicyId,
    PreflightFeature, PreflightModerator, build_moderator_from_config, load_agent_config,
};
pub use prompt_advisor::{AdvisorIssue, DynamicSnapshot, PromptAdvice, PromptAdvisor};
pub use provider_info::LlmInfo;
#[cfg(feature = "llama-cpp")]
pub use provider_protected::{ProtectedLoadError, recover_payload_key};
pub use rlm::{
    RlmConfig, RlmContextManager, RlmDecompositionResult, RlmProbeResult, RlmSearchResult,
    RlmStats, SharedRlmManager, create_rlm_manager, create_rlm_manager_with_config,
};
pub use selector::{ModelSelector, ProviderAvailability};
pub use stream::{RouteMetadata, RouteResponse, RouteStream, StreamChunk};
pub use validator::{ResponseValidator, ValidationError};

#[cfg(feature = "tdf-encrypt")]
pub use tdf_audit::{MessageEncryptor, TdfAuditConfig};

// Re-export response processing types
pub use response::{sanitize_response, strip_think_blocks, strip_tool_blocks};

/// Extract search keywords from a user message (for tool search telemetry).
pub fn tool_search_keywords(text: &str) -> String {
    tool_extraction::extract_keywords(text)
}

pub use learning::{
    AgentContribution, AgentUtility, AgentUtilityStats, BetaPrior, BurstFeedback, FinalTaskReport,
    LearningConfig, LearningModule, QualityMetrics,
};
pub use spec_stats::SpecStats;

/// Structured events emitted by the router for telemetry consumers.
///
/// Prefer `drain_events()` over log lines — callers aggregate these into
/// metrics or UI events instead of scraping logs.
#[derive(Debug, Clone)]
pub enum RouterEvent {
    /// A model's rolling spec-decoding accept rate dropped below the threshold.
    ///
    /// The router will stop recommending spec decoding for this model until its
    /// accept rate recovers. Emitted exactly once per low→high→low transition.
    SpecDecodingDisabled {
        model: String,
        accept_rate_pct: u32,
        sample_size: u32,
    },
    /// A local answer visibly collapsed and a cloud upgrade was offered, but the
    /// cloud policy did not authorize silent spend, so the router stayed local.
    ///
    /// This is the quality→spend boundary made observable: the collapse (a
    /// quality signal) requested an upgrade, and the spend plane refused to
    /// spend without asking. Operators see how often local-first holds.
    CloudEscalationBlocked {
        /// What triggered the refused escalation, e.g. `collapse:RepetitionLoop`.
        reason: String,
        /// The cloud policy that refused, e.g. `AskBeforeCloud`.
        policy: String,
    },
    /// The feasibility plane reported a non-nominal local runtime condition.
    ///
    /// Pre-dispatch this surfaces a reshape need (prompt does not fit the
    /// context) or unavailability; post-dispatch it surfaces degraded
    /// throughput for the model+context configuration. It is a *cost/feasibility*
    /// signal only — it never authorizes cloud spend. Consumers (UI, the RLM
    /// chunking layer) decide what to do with it.
    LocalFeasibility {
        model: String,
        /// The `FeasibilityVerdict` as a debug string, e.g. `LocalNeedsChunking`.
        verdict: String,
        prompt_tokens: u32,
        n_ctx: u32,
        /// Observed decode throughput (tokens/sec); `None` for the pre-dispatch
        /// structural check, `Some` for the post-dispatch throughput sample.
        tokens_per_sec: Option<f32>,
    },
    /// A local answer collapsed and the cloud policy *permits* cloud but
    /// requires confirmation (`AskBeforeCloud`). The router stayed local and
    /// surfaced this offer so the caller (CLI / UI) can prompt the user, then
    /// `confirm_next_cloud_upgrade()` + re-dispatch to escalate. This is the
    /// "ask before cloud" handshake made observable.
    CloudUpgradeOffered {
        /// Why the upgrade is offered, e.g. `LocalCollapsed`.
        reason: String,
        /// Projected cloud cost in cents.
        projected_cost_cents: u64,
    },
}

impl RouterEvent {
    /// Stable snake_case kind, used as the observability counter key.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::SpecDecodingDisabled { .. } => "spec_decoding_disabled",
            Self::CloudEscalationBlocked { .. } => "cloud_escalation_blocked",
            Self::LocalFeasibility { .. } => "local_feasibility",
            Self::CloudUpgradeOffered { .. } => "cloud_upgrade_offered",
        }
    }
}

#[cfg(feature = "llama-cpp")]
use arkavo_identity::IdentitySession;
#[cfg(feature = "llama-cpp")]
use arkavo_llm::ModelRegistry;
use arkavo_llm::{Message, Role};
use arkavo_mcp_tools::ToolRegistry;
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};

/// Base cooldown after first availability failure (5 minutes).
/// Doubles on each consecutive failure: 5 → 10 → 20 → 40 → 60 (capped).
/// Resets to base after the first successful response from that model.
const MODEL_COOLDOWN_BASE_SECS: u64 = 300;

/// Maximum cooldown duration regardless of consecutive failures (1 hour).
const MODEL_COOLDOWN_MAX_SECS: u64 = 3600;

/// Short cooldown for quality failures (timeout, no tool calls).
/// Shorter than availability cooldown to allow faster model rotation.
const MODEL_QUALITY_COOLDOWN_BASE_SECS: u64 = 30;

/// Intelligent router for cost-optimized model selection
pub struct Router {
    classifier: Arc<TaskClassifier>,
    selector: Arc<ModelSelector>,
    model_learning: Arc<LearningModule>,
    /// Cached local models — loaded once, reused across requests.
    #[cfg(feature = "llama-cpp")]
    model_registry: Arc<ModelRegistry>,
    /// OIDC session used to unwrap `.gguf.tdf` models through platform KAS.
    #[cfg(feature = "llama-cpp")]
    identity: Arc<IdentitySession>,
    /// Temporarily excluded models: name → (when, consecutive_failures).
    /// Cooldown duration doubles each consecutive failure, resets on success.
    model_cooldowns: Arc<RwLock<std::collections::HashMap<String, (std::time::Instant, u32)>>>,
    metrics: Arc<RwLock<RoutingMetrics>>,
    connectivity: Arc<ConnectivityChecker>,
    offline_mode: bool,
    preflight: Option<Arc<preflight::PreflightModerator>>,
    advisor: Arc<PromptAdvisor>,
    #[cfg(feature = "critic")]
    critic: Option<Arc<arkavo_critic::CriticPipeline>>,
    #[cfg(feature = "advisor-persistence")]
    advisor_store: Option<Arc<arkavo_memory::AdvisorStateStore>>,
    #[cfg(feature = "advisor-persistence")]
    advisor_persist_count: std::sync::atomic::AtomicU64,
    #[cfg(feature = "advisor-persistence")]
    advisor_last_persist: std::sync::Mutex<std::time::Instant>,
    #[cfg(feature = "tdf-encrypt")]
    tdf_encryptor: Option<Arc<tdf_audit::MessageEncryptor>>,
    #[cfg(feature = "tdf-encrypt")]
    tdf_audit_store: Option<Arc<arkavo_memory::TdfAuditStore>>,
    /// Serializes concurrent LLM inference calls for task/orchestrator work.
    /// Local models (llama-cpp) share a single KV cache and cannot handle
    /// concurrent context allocation. This semaphore queues requests so the
    /// second caller waits instead of failing with an OOM/slot error.
    inference_semaphore: Arc<Semaphore>,
    /// Separate semaphore for chat inference so chat never blocks game ticks.
    /// Uses the fastest local model (0.8B/3B) which has its own context pool.
    chat_semaphore: Arc<Semaphore>,
    /// Separate semaphore for synthesis/internal tasks (route_fast).
    /// Prevents synthesis from blocking production inference since they
    /// use different models (fastest_local_model vs Thompson-selected).
    synthesis_semaphore: Arc<Semaphore>,
    /// Runtime-updateable optimal inference configs per model.
    /// Seeded from compile-time defaults, updated via autoresearch sweeps and gossip.
    pub optimal_configs: Arc<optimal_config::OptimalConfigStore>,
    /// Tracks which model was last selected by route_with_tools().
    /// The conductor reads this after tool execution to attribute
    /// reward-based corrective feedback to the right Thompson Sampling prior.
    last_routed_model: Arc<std::sync::RwLock<Option<String>>>,
    /// Last routing decision trace for downstream attribution
    last_decision_trace: Arc<std::sync::RwLock<Option<learning::DecisionTrace>>>,
    /// Recent decision traces for UI dashboard (ring buffer, max 50)
    recent_traces: Arc<std::sync::RwLock<std::collections::VecDeque<learning::DecisionTrace>>>,
    /// Consecutive negative-reward tick count per model. Unlike cooldown_consecutive
    /// (reset on successful inference), this is only reset when rewards turn positive.
    /// Used to trigger model hint release after sustained poor task quality.
    reward_failure_counts: Arc<tokio::sync::RwLock<std::collections::HashMap<String, u32>>>,
    /// Per-model rolling spec-decoding accept-rate tracker.
    /// Decides whether to enable NGRAM spec for the next request to each model.
    spec_stats: Arc<spec_stats::SpecStats>,
    /// Structured router events waiting to be consumed by the caller.
    /// Callers drain these via `drain_events()` instead of scraping log lines.
    pending_events: Arc<std::sync::Mutex<Vec<RouterEvent>>>,
    /// Spend-plane posture. Decides whether a feasibility/quality re-route may
    /// cross into paid cloud. Defaults to `AskBeforeCloud`, so an availability
    /// failure or a quality collapse never silently spends — it stays local.
    cloud_policy: arkavo_budget::CloudPolicy,
    /// Per-config runtime-cost baseline for the feasibility plane. Learns each
    /// model+context's decode throughput from real `InferenceTiming` so "slow"
    /// is judged relative to that configuration, not an absolute threshold.
    feasibility_baseline: Arc<planes::FeasibilityBaseline>,
    /// Optional live budget tracker (spend plane). When present, the loop's
    /// cloud-escalation decision consults the real remaining cap via
    /// `authorize_cloud_spend`; when absent there is no cap to enforce.
    budget_tracker: Option<Arc<arkavo_budget::BudgetTracker>>,
    /// Authored per-MTok pricing registry (spend plane). When non-empty,
    /// `projected_cloud_cost` prices a cloud arm from these authored rates
    /// (the manifest is the pricing home); when empty it falls back to the
    /// built-in static per-model estimate. Behind an `Arc<RwLock>` so a
    /// specialization bundle can replace it live without rebuilding the
    /// Router (which is shared across handlers via `Arc`). Populated from a
    /// specialization bundle's `manifest_pricing` via [`Router::with_pricing`]
    /// (builder) or [`Router::apply_manifest_pricing`] (live update). Uses a
    /// std `RwLock` (not tokio) so `projected_cloud_cost` can read it without
    /// `.await` on the (rare) collapse-driven upgrade path.
    pricing: Arc<std::sync::RwLock<arkavo_budget::provider_costs::ProviderPricing>>,
    /// One-shot user confirmation for the next cloud upgrade under
    /// `AskBeforeCloud`. The caller sets it via `confirm_next_cloud_upgrade()`
    /// after the user approves a `CloudUpgradeOffered`; the loop consumes it
    /// when authorizing spend.
    cloud_confirmation: std::sync::atomic::AtomicBool,
}

impl Router {
    pub async fn new() -> Result<Self> {
        let selector = Arc::new(ModelSelector::new());
        let model_learning = Arc::new(LearningModule::new());
        selector_quality::seed_model_learning(&selector, &model_learning).await;

        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector,
            model_learning,
            #[cfg(feature = "llama-cpp")]
            model_registry: Arc::new(ModelRegistry::new()),
            #[cfg(feature = "llama-cpp")]
            identity: Arc::new(IdentitySession::new()),
            model_cooldowns: Arc::new(RwLock::new(std::collections::HashMap::new())),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            connectivity: Arc::new(ConnectivityChecker::new()),
            offline_mode: false,
            preflight: None,
            advisor: Arc::new(PromptAdvisor::new()),
            #[cfg(feature = "critic")]
            critic: None,
            #[cfg(feature = "advisor-persistence")]
            advisor_store: None,
            #[cfg(feature = "advisor-persistence")]
            advisor_persist_count: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "advisor-persistence")]
            advisor_last_persist: std::sync::Mutex::new(std::time::Instant::now()),
            #[cfg(feature = "tdf-encrypt")]
            tdf_encryptor: None,
            #[cfg(feature = "tdf-encrypt")]
            tdf_audit_store: None,
            optimal_configs: Arc::new(optimal_config::OptimalConfigStore::new()),
            inference_semaphore: Arc::new(Semaphore::new(1)),
            chat_semaphore: Arc::new(Semaphore::new(1)),
            synthesis_semaphore: Arc::new(Semaphore::new(1)),
            last_routed_model: Arc::new(std::sync::RwLock::new(None)),
            last_decision_trace: Arc::new(std::sync::RwLock::new(None)),
            recent_traces: Arc::new(std::sync::RwLock::new(std::collections::VecDeque::new())),
            reward_failure_counts: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            spec_stats: Arc::new(spec_stats::SpecStats::default()),
            pending_events: Arc::new(std::sync::Mutex::new(Vec::new())),
            cloud_policy: arkavo_budget::CloudPolicy::default(),
            feasibility_baseline: Arc::new(
                planes::FeasibilityBaseline::default_path()
                    .map(planes::FeasibilityBaseline::load)
                    .unwrap_or_default(),
            ),
            budget_tracker: None,
            pricing: Arc::new(std::sync::RwLock::new(
                arkavo_budget::provider_costs::ProviderPricing::new(),
            )),
            cloud_confirmation: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub async fn new_offline() -> Result<Self> {
        let selector = Arc::new(ModelSelector::new());
        let model_learning = Arc::new(LearningModule::new());
        selector_quality::seed_model_learning(&selector, &model_learning).await;

        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector,
            model_learning,
            #[cfg(feature = "llama-cpp")]
            model_registry: Arc::new(ModelRegistry::new()),
            #[cfg(feature = "llama-cpp")]
            identity: Arc::new(IdentitySession::new()),
            model_cooldowns: Arc::new(RwLock::new(std::collections::HashMap::new())),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            connectivity: Arc::new(ConnectivityChecker::new()),
            offline_mode: true,
            preflight: None,
            advisor: Arc::new(PromptAdvisor::new()),
            #[cfg(feature = "critic")]
            critic: None,
            #[cfg(feature = "advisor-persistence")]
            advisor_store: None,
            #[cfg(feature = "advisor-persistence")]
            advisor_persist_count: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "advisor-persistence")]
            advisor_last_persist: std::sync::Mutex::new(std::time::Instant::now()),
            #[cfg(feature = "tdf-encrypt")]
            tdf_encryptor: None,
            #[cfg(feature = "tdf-encrypt")]
            tdf_audit_store: None,
            optimal_configs: Arc::new(optimal_config::OptimalConfigStore::new()),
            inference_semaphore: Arc::new(Semaphore::new(1)),
            chat_semaphore: Arc::new(Semaphore::new(1)),
            synthesis_semaphore: Arc::new(Semaphore::new(1)),
            last_routed_model: Arc::new(std::sync::RwLock::new(None)),
            last_decision_trace: Arc::new(std::sync::RwLock::new(None)),
            recent_traces: Arc::new(std::sync::RwLock::new(std::collections::VecDeque::new())),
            reward_failure_counts: Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            spec_stats: Arc::new(spec_stats::SpecStats::default()),
            pending_events: Arc::new(std::sync::Mutex::new(Vec::new())),
            cloud_policy: arkavo_budget::CloudPolicy::default(),
            feasibility_baseline: Arc::new(
                planes::FeasibilityBaseline::default_path()
                    .map(planes::FeasibilityBaseline::load)
                    .unwrap_or_default(),
            ),
            budget_tracker: None,
            pricing: Arc::new(std::sync::RwLock::new(
                arkavo_budget::provider_costs::ProviderPricing::new(),
            )),
            cloud_confirmation: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Add pre-flight moderation to the router
    ///
    /// Pre-flight moderation evaluates TØR-G circuits against requests
    /// BEFORE LLM inference, blocking policy-violating requests early.
    #[must_use]
    pub fn with_preflight(mut self, moderator: preflight::PreflightModerator) -> Self {
        self.preflight = Some(Arc::new(moderator));
        self
    }

    /// Set the spend-plane cloud policy (default `AskBeforeCloud`).
    ///
    /// Governs whether a feasibility/quality re-route may cross into paid
    /// cloud. Only `CloudWithinCap` permits silent cloud spend; the other
    /// postures keep an availability failure or quality collapse on a local
    /// model.
    #[must_use]
    pub fn with_cloud_policy(mut self, policy: arkavo_budget::CloudPolicy) -> Self {
        self.cloud_policy = policy;
        self
    }

    /// The configured spend-plane cloud policy.
    pub fn cloud_policy(&self) -> arkavo_budget::CloudPolicy {
        self.cloud_policy
    }

    /// Grant one-shot user confirmation for the next cloud upgrade.
    ///
    /// Call this after the user approves a `CloudUpgradeOffered` event, then
    /// re-dispatch the request: under `AskBeforeCloud` the next collapse-driven
    /// escalation will be authorized (within the budget cap). The flag is
    /// consumed by the first routing decision that consults it.
    pub fn confirm_next_cloud_upgrade(&self) {
        self.cloud_confirmation
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Consume the one-shot cloud confirmation (read-and-clear).
    fn consume_cloud_confirmation(&self) -> bool {
        self.cloud_confirmation
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    }

    /// Attach a live budget tracker so the loop's cloud-escalation decision
    /// enforces the real remaining cap (spend plane), not just the policy gate.
    #[must_use]
    pub fn with_budget_tracker(mut self, tracker: Arc<arkavo_budget::BudgetTracker>) -> Self {
        self.budget_tracker = Some(tracker);
        self
    }

    /// Install authored per-MTok pricing as the cost source for the spend
    /// plane's projected-cost gate. When a cloud arm is present in the
    /// registry, its authored rate is authoritative; otherwise the built-in
    /// static estimate is used. Typically populated from a specialization
    /// bundle's `manifest_pricing` so distributed agents share one trusted
    /// cost model sourced from the signed manifest.
    pub fn with_pricing(mut self, pricing: arkavo_budget::provider_costs::ProviderPricing) -> Self {
        self.pricing = Arc::new(std::sync::RwLock::new(pricing));
        self
    }

    /// Replace the live cost gate's pricing registry in place. Used when a
    /// specialization bundle arrives after the Router is already running: the
    /// bundle's `manifest_pricing` (trusted, TDF-delivered config) becomes the
    /// authoritative source for `projected_cloud_cost`. Safe to call from a
    /// shared `&Arc<Router>` because pricing is behind an `RwLock`.
    pub fn apply_manifest_pricing(&self, pricing: arkavo_budget::provider_costs::ProviderPricing) {
        // Recover the guard on poison: the inner data is still valid after a
        // panic elsewhere, and a pricing assignment can never itself panic, so
        // we always want the update to land rather than silently leaving the
        // gate on the stale/static table.
        let mut guard = match self.pricing.write() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        *guard = pricing;
    }

    /// Spend caps for a cloud-escalation decision, read from the live budget
    /// tracker. Without a tracker (or a session limit) there is no cap to
    /// enforce, so the remaining cap is reported as effectively unbounded — the
    /// policy gate in `authorize_cloud_spend` still applies.
    async fn cloud_spend_caps(&self) -> arkavo_budget::SpendCaps {
        let unbounded = arkavo_budget::TokenCost::from_dollars(f64::from(u32::MAX));
        let remaining_cap = match &self.budget_tracker {
            Some(tracker) => {
                let status = tracker.get_status().await;
                status
                    .session_limit
                    .map(|limit| {
                        limit
                            .checked_sub(status.session_spent)
                            .unwrap_or(arkavo_budget::TokenCost::ZERO)
                    })
                    .unwrap_or(unbounded)
            }
            None => unbounded,
        };
        arkavo_budget::SpendCaps {
            remaining_cap,
            per_request_max: None,
        }
    }

    /// Projected cost of escalating this decision to cloud, used by the spend
    /// plane's cap check. Estimates against the first cloud arm in the decision's
    /// fallback chain (or a cheap default), since the concrete arm is only
    /// chosen after the spend gate passes.
    ///
    /// Pricing source: authored registry first (`with_pricing`), falling back to
    /// the built-in static per-model estimate when the arm is absent from the
    /// registry. This makes authored manifest rates authoritative for the live
    /// gate (#635) while keeping a safe static fallback for unknown models.
    ///
    /// Naming contract: the manifest's `provider`/`model_id` must match the
    /// router's `ModelChoice::provider()`/`name()` strings exactly (e.g.
    /// `google` / `gemini-flash-latest`). A mismatch is not an error — the arm
    /// falls back to the static estimate — but it is logged so operators can
    /// detect that an authored rate silently did not apply.
    fn projected_cloud_cost(&self, decision: &RoutingDecision) -> arkavo_budget::TokenCost {
        let arm = decision
            .fallback_chain
            .iter()
            .find(|m| m.is_cloud())
            .cloned()
            .unwrap_or(ModelChoice::GeminiFlash);
        let est = decision.task_category.estimated_tokens();
        let (authored, registry_populated) = self
            .pricing
            .read()
            .map(|p| {
                (
                    p.estimate_cost(arm.provider(), arm.name(), est.input, est.output),
                    p.model_count() > 0,
                )
            })
            .unwrap_or((None, false));
        authored.unwrap_or_else(|| {
            if registry_populated {
                // Authored table present but this arm isn't in it: a
                // provider/model-id naming mismatch between the manifest and
                // ModelChoice. Without this log the gate silently reverts to
                // the static estimate — exactly what #635 set out to fix.
                tracing::warn!(
                    provider = arm.provider(),
                    model = arm.name(),
                    "authored pricing table present but model not found; \
                     falling back to static estimate. Manifest model_id/provider \
                     must match ModelChoice::name()/provider() exactly."
                );
            }
            arkavo_budget::TokenCost::from_dollars(RoutingDecision::estimate_cost(
                &arm,
                decision.task_category,
            ))
        })
    }

    /// Exclusion set for a feasibility/quality-driven re-route.
    ///
    /// Starts from the cooldown-excluded models, then applies the spend plane:
    /// unless the cloud policy authorizes silent spend, all cloud arms are
    /// excluded so re-selection stays local. This is how the routing loop keeps
    /// an availability failure or a quality collapse from silently spending.
    async fn reroute_exclusions(&self) -> Vec<String> {
        let excluded = self.get_excluded_models().await;
        planes::augment_exclusions_for_policy(excluded, self.cloud_policy)
    }

    /// Free KV-cache slots for a local model, read from the live context pool.
    /// Defaults to 1 ("assume placeable") when the pool has no entry or on
    /// builds without the local engine — an honest fallback, not a fake count.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    fn kv_slots_free(&self, model_name: &str) -> usize {
        self.model_registry
            .context_pool()
            .stats(model_name)
            .map(|s| s.available)
            .unwrap_or(1)
    }

    #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
    fn kv_slots_free(&self, _model_name: &str) -> usize {
        1
    }

    /// Pre-dispatch feasibility check for a local model (the feasibility plane).
    ///
    /// Assembles live `RuntimeStats` (prompt estimate, model context window,
    /// KV-cache availability) and classifies. When the prompt does not fit
    /// (reshape) or local cannot run, it emits a `LocalFeasibility` event so the
    /// UI / RLM chunking layer can react. Cloud models are skipped, and this
    /// never authorizes spend — it only surfaces a cost/feasibility signal.
    fn check_local_feasibility(&self, model: &ModelChoice, prompt_tokens: u32) {
        if !model.is_local() {
            return;
        }
        let n_ctx =
            arkavo_context::rlm_detection::model_context_size(Some(model.name()), false) as u32;
        let stats = RuntimeStats {
            prompt_tokens,
            n_ctx,
            local_model_available: self.is_model_available(model),
            kv_cache_slots_free: self.kv_slots_free(model.name()),
            tokens_per_sec: None,
            context_overflow: false,
        };
        let key = FeasibilityBaseline::key(model.name(), n_ctx);
        let verdict = self.feasibility_baseline.classify(&stats, &key);
        if verdict.needs_reshape() || verdict == FeasibilityVerdict::LocalCannotRun {
            self.emit_event(RouterEvent::LocalFeasibility {
                model: model.name().to_string(),
                verdict: format!("{verdict:?}"),
                prompt_tokens,
                n_ctx,
                tokens_per_sec: None,
            });
        }
    }

    /// Post-dispatch: record observed decode throughput for a local model into
    /// the per-config baseline, and emit a `LocalFeasibility` event when the
    /// sample is slow *for that configuration*. Feasibility telemetry only —
    /// it learns "slow" per model+context rather than from an absolute floor,
    /// and never authorizes spend.
    fn record_local_throughput(
        &self,
        model: &ModelChoice,
        timing: &arkavo_llm::InferenceTiming,
        prompt_tokens: u32,
    ) {
        if !model.is_local() || timing.generation_ms <= 0.0 || timing.n_eval == 0 {
            return;
        }
        let tps = (f64::from(timing.n_eval) / (timing.generation_ms / 1000.0)) as f32;
        let n_ctx =
            arkavo_context::rlm_detection::model_context_size(Some(model.name()), false) as u32;
        let key = FeasibilityBaseline::key(model.name(), n_ctx);
        // Judge against prior history before folding this sample in.
        let slow = self.feasibility_baseline.is_slow(&key, tps) == Some(true);
        self.feasibility_baseline.record(&key, tps);
        if slow {
            self.emit_event(RouterEvent::LocalFeasibility {
                model: model.name().to_string(),
                verdict: format!("{:?}", FeasibilityVerdict::LocalCanRunSlowly),
                prompt_tokens,
                n_ctx,
                tokens_per_sec: Some(tps),
            });
        }
    }

    /// Run preflight moderation check without full classification.
    ///
    /// Returns `None` if no preflight moderator is configured (allows all).
    /// Returns `Some(result)` with the moderation outcome otherwise.
    pub fn check_preflight(&self, input: &str) -> Option<preflight::ModerationResult> {
        self.preflight.as_ref().map(|pf| pf.check(input))
    }

    /// Add post-LLM critic validation to the router
    ///
    /// The CriticPipeline validates LLM responses AFTER inference,
    /// checking for policy violations, schema errors, and semantic issues.
    #[cfg(feature = "critic")]
    #[must_use]
    pub fn with_critic(mut self, pipeline: arkavo_critic::CriticPipeline) -> Self {
        self.critic = Some(Arc::new(pipeline));
        self
    }

    /// Attach an advisor state store for persisting learned adjustments.
    ///
    /// Loads all previously persisted adjustments and imports them into
    /// the in-memory advisor. The store is then retained for runtime saves.
    #[cfg(feature = "advisor-persistence")]
    #[must_use]
    pub async fn with_advisor_store(mut self, store: arkavo_memory::AdvisorStateStore) -> Self {
        if let Ok(persisted) = store.load_all().await {
            let snapshots: Vec<prompt_advisor::DynamicSnapshot> = persisted
                .into_iter()
                .filter_map(|p| {
                    let issue = match p.issue.as_str() {
                        "UnwantedCodeFence" => AdvisorIssue::UnwantedCodeFence,
                        "OutputLoop" => AdvisorIssue::OutputLoop,
                        "WrongExpert" => AdvisorIssue::WrongExpert,
                        "Timeout" => AdvisorIssue::Timeout,
                        "ToolError" => AdvisorIssue::ToolError,
                        "NoToolCalls" => AdvisorIssue::NoToolCalls,
                        _ => return None,
                    };
                    Some(prompt_advisor::DynamicSnapshot {
                        label: p.label,
                        model_family: p.model_family,
                        issue,
                        text: p.text,
                        success_rate: p.success_rate,
                        applications: p.applications,
                        feedback_count: p.feedback_count,
                    })
                })
                .collect();

            if !snapshots.is_empty() {
                tracing::info!("Loaded {} persisted advisor adjustments", snapshots.len());
                self.advisor.import_dynamic(snapshots);
            }
        }

        self.advisor_store = Some(Arc::new(store));
        self
    }

    /// Attach a TDF encryptor for cloud-bound prompt audit.
    #[cfg(feature = "tdf-encrypt")]
    #[must_use]
    pub fn with_tdf_encryptor(mut self, encryptor: tdf_audit::MessageEncryptor) -> Self {
        self.tdf_encryptor = Some(Arc::new(encryptor));
        self
    }

    /// Attach a TDF audit store for persisting encryption manifests.
    #[cfg(feature = "tdf-encrypt")]
    #[must_use]
    pub fn with_tdf_audit_store(mut self, store: arkavo_memory::TdfAuditStore) -> Self {
        self.tdf_audit_store = Some(Arc::new(store));
        self
    }

    /// Get a reference to the prompt advisor
    pub fn advisor(&self) -> &PromptAdvisor {
        &self.advisor
    }

    /// Get a reference to the model learning module (Thompson Sampling state)
    pub fn model_learning(&self) -> &LearningModule {
        &self.model_learning
    }

    /// Get a reference to the per-model spec-decoding accept-rate tracker.
    ///
    /// Callers feed `InferenceTiming.n_draft`/`n_accepted` back here after
    /// each completion so the router learns which models benefit from spec.
    pub fn spec_stats(&self) -> &Arc<spec_stats::SpecStats> {
        &self.spec_stats
    }

    /// Decide whether the next request to `model_name` should use spec
    /// decoding, and emit a `SpecDecodingDisabled` event on the first
    /// below-threshold crossing.
    ///
    /// All call sites that consult `spec_stats.decide(...)` must go through
    /// this helper. Calling `decide()` directly silently discards the
    /// `crossed_below_threshold` signal — the threshold-crossing event would
    /// never reach the `pending_events` queue, so operators would never see
    /// that spec was auto-disabled for a model.
    pub(crate) fn decide_spec_with_event(&self, model_name: &str) -> bool {
        let decision = self.spec_stats.decide(model_name);
        if let Some(rate_pct) = decision.crossed_below_threshold {
            let sample_size = self.spec_stats.window();
            self.emit_event(RouterEvent::SpecDecodingDisabled {
                model: model_name.to_string(),
                accept_rate_pct: rate_pct,
                sample_size,
            });
        }
        decision.use_spec
    }

    /// Enqueue a structured router event for telemetry consumers.
    ///
    /// Capped at 1024 pending events to bound growth when no `drain_events()`
    /// consumer is polling.
    pub(crate) fn emit_event(&self, event: RouterEvent) {
        // Backend sink: count by kind at emit time so events are observable
        // even when no `drain_events()` consumer (e.g. the AG-UI gateway) runs.
        arkavo_observability::event_counters::global_event_counters().increment(event.kind());
        if let Ok(mut g) = self.pending_events.lock()
            && g.len() < 1024
        {
            g.push(event);
        }
    }

    /// Drain all pending structured router events.
    ///
    /// Returns all events accumulated since the last call and clears the queue.
    /// Callers should poll this after each routing cycle and forward events to
    /// their telemetry aggregator.
    pub fn drain_events(&self) -> Vec<RouterEvent> {
        self.pending_events
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default()
    }

    /// Set per-agent memory budget on the model selector.
    ///
    /// Models whose weight files exceed this limit are excluded from the
    /// feasible set, preventing Thompson Sampling from loading oversized
    /// models into the registry.
    pub fn set_memory_budget(&self, bytes: u64) {
        self.selector.set_memory_budget(bytes);
    }

    /// Get the model name last selected by `route_with_tools()`.
    ///
    /// Returns `None` if no routing has occurred yet. Used by the conductor
    /// to attribute reward-based corrective feedback to the right model.
    pub fn last_routed_model(&self) -> Option<String> {
        self.last_routed_model.read().ok().and_then(|g| g.clone())
    }

    /// Get the trace from the last routing decision.
    /// Used by the conductor to attribute trace IDs to tool call events.
    pub fn last_decision_trace(&self) -> Option<learning::DecisionTrace> {
        self.last_decision_trace.read().ok().and_then(|g| g.clone())
    }

    /// Get recent decision traces for the UI dashboard.
    pub fn recent_decision_traces(&self, limit: usize) -> Vec<learning::DecisionTrace> {
        self.recent_traces
            .read()
            .ok()
            .map(|g| g.iter().rev().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    /// Extract model family from a model name (e.g., "glm-4.7-flash" → "glm").
    ///
    /// Simple heuristic: take the first segment before any dash-digit pattern.
    pub fn detect_model_family(model_name: &str) -> String {
        let lower = model_name.to_lowercase();
        // Common model family prefixes
        for family in &[
            "glm",
            "qwen",
            "gemma",
            "ministral",
            "mistral",
            "deepseek",
            "llama",
            "phi",
        ] {
            if lower.starts_with(family) {
                return (*family).to_string();
            }
        }
        // Fallback: take first segment before '-'
        lower.split('-').next().unwrap_or(&lower).to_string()
    }

    /// Log the full Thompson Sampling state for all tracked models.
    ///
    /// Shows Beta prior (α, β), expected value, and observation count
    /// per (model, category) pair. Useful for diagnostics after a mesh run.
    pub async fn dump_learning_state(&self) {
        let models = self.selector.feasible_models();
        tracing::info!("=== Thompson Sampling State ===");
        for model in &models {
            let stats = self.model_learning.get_category_stats(model.name()).await;
            if stats.is_empty() {
                tracing::info!(model = model.name(), "No learning data");
                continue;
            }
            for (category, alpha, beta, ev, obs) in &stats {
                tracing::info!(
                    model = model.name(),
                    category = category.as_str(),
                    alpha = format!("{alpha:.2}").as_str(),
                    beta = format!("{beta:.2}").as_str(),
                    expected = format!("{ev:.3}").as_str(),
                    observations = obs,
                    "Prior state"
                );
            }
        }
        tracing::info!("=== End Thompson Sampling State ===");
    }

    /// Record that a model is temporarily unavailable (availability failure, not quality).
    ///
    /// This does NOT update the model's Beta prior — availability failures are
    /// operational events, not learning events. Cooldown duration doubles on
    /// each consecutive failure (5 → 10 → 20 → 40 → 60 min cap), then resets
    /// after the first successful response from that model.
    pub async fn record_model_cooldown(&self, model_name: &str) {
        let mut cooldowns = self.model_cooldowns.write().await;
        let consecutive = cooldowns
            .get(model_name)
            .map(|(_, count)| count + 1)
            .unwrap_or(1);
        cooldowns.insert(
            model_name.to_string(),
            (std::time::Instant::now(), consecutive),
        );
        let duration = Self::cooldown_duration_secs(consecutive);
        tracing::info!(
            model = model_name,
            consecutive,
            "Model cooled down for {}s (availability failure)",
            duration
        );
    }

    /// Record a quality-based cooldown (timeout, no tool calls).
    ///
    /// Uses a shorter base (30s) than availability cooldown (5min) to allow
    /// faster model rotation while still breaking retry loops. Progression:
    /// 30s → 60s → 120s → 240s → ... → 3600s cap.
    pub async fn record_quality_cooldown(&self, model_name: &str) {
        let mut cooldowns = self.model_cooldowns.write().await;
        let consecutive = cooldowns
            .get(model_name)
            .map(|(_, count)| count + 1)
            .unwrap_or(1);
        cooldowns.insert(
            model_name.to_string(),
            (std::time::Instant::now(), consecutive),
        );
        let duration = Self::quality_cooldown_duration_secs(consecutive);
        tracing::info!(
            model = model_name,
            consecutive,
            "Model cooled down for {duration}s (quality failure)"
        );
    }

    /// Quality cooldown duration: 30s base, capped at 60s.
    ///
    /// Quality failures (timeout, no tool calls) are transient — the purpose
    /// of cooldown is to rotate to another model, not to exile a model.
    /// Aggressive escalation causes a death spiral in multi-agent meshes
    /// where all models end up cooled down simultaneously.
    fn quality_cooldown_duration_secs(consecutive: u32) -> u64 {
        let shift = consecutive.saturating_sub(1).min(1); // cap at 2^1 = 2x
        let multiplier = 1u64 << shift;
        MODEL_QUALITY_COOLDOWN_BASE_SECS
            .saturating_mul(multiplier)
            .min(60)
    }

    /// Clear cooldown for a model after a successful response.
    ///
    /// Resets the exponential backoff so the next failure starts at the base duration.
    async fn clear_model_cooldown(&self, model_name: &str) {
        self.model_cooldowns.write().await.remove(model_name);
    }

    /// Cooldown duration with exponential backoff: base × 2^(n-1), capped.
    fn cooldown_duration_secs(consecutive: u32) -> u64 {
        let shift = consecutive.saturating_sub(1).min(10); // prevent overflow
        let multiplier = 1u64 << shift;
        MODEL_COOLDOWN_BASE_SECS
            .saturating_mul(multiplier)
            .min(MODEL_COOLDOWN_MAX_SECS)
    }

    /// Get the consecutive failure count for a model (0 if not on cooldown).
    pub async fn get_cooldown_consecutive(&self, model_name: &str) -> u32 {
        let cooldowns = self.model_cooldowns.read().await;
        cooldowns
            .get(model_name)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    }

    /// Record a negative-reward tick for a model. Unlike cooldown_consecutive,
    /// this counter is only reset when rewards turn positive — not on successful inference.
    pub async fn record_reward_failure(&self, model_name: &str) {
        let mut counts = self.reward_failure_counts.write().await;
        let count = counts.entry(model_name.to_string()).or_insert(0);
        *count += 1;
        tracing::info!(
            model = model_name,
            consecutive = *count,
            "Reward failure recorded"
        );
    }

    /// Clear the reward failure counter when rewards turn positive.
    pub async fn clear_reward_failure(&self, model_name: &str) {
        let mut counts = self.reward_failure_counts.write().await;
        if counts.remove(model_name).is_some() {
            tracing::info!(model = model_name, "Reward failure counter cleared");
        }
    }

    /// Get the consecutive negative-reward tick count for a model.
    pub async fn get_reward_failure_count(&self, model_name: &str) -> u32 {
        let counts = self.reward_failure_counts.read().await;
        counts.get(model_name).copied().unwrap_or(0)
    }

    /// Get model names currently on cooldown (expired entries are pruned)
    pub async fn get_excluded_models(&self) -> Vec<String> {
        let mut cooldowns = self.model_cooldowns.write().await;
        cooldowns.retain(|_, (since, consecutive)| {
            let duration =
                std::time::Duration::from_secs(Self::cooldown_duration_secs(*consecutive));
            since.elapsed() < duration
        });
        let excluded: Vec<String> = cooldowns.keys().cloned().collect();
        if !excluded.is_empty() {
            tracing::info!(
                excluded = %excluded.join(", "),
                "Models on cooldown (excluded from selection)"
            );
        }
        excluded
    }

    pub fn set_offline_mode(&mut self, offline: bool) {
        self.offline_mode = offline;
    }

    pub async fn check_connectivity(&self) -> bool {
        self.connectivity.is_online().await
    }

    /// Get routing decision without executing (for callers that just need model selection)
    pub async fn classify(&self, task_description: &str) -> Result<RoutingDecision> {
        let classification = self.classifier.classify(task_description).await?;

        tracing::info!(
            category = ?classification.category,
            confidence = classification.confidence,
            "Task classified"
        );

        let excluded = self.get_excluded_models().await;
        let mut decision = self
            .selector
            .select_adaptive(&self.model_learning, &classification, 0.0, &excluded)
            .await?;

        tracing::info!(
            model = %decision.recommended_model.name(),
            reasoning = %decision.reasoning,
            "Model selected"
        );

        if (self.offline_mode || !self.connectivity.is_online().await)
            && decision.recommended_model.is_cloud()
        {
            let local_model = self.get_local_fallback(classification.category);

            decision.reasoning = format!(
                "Offline mode: Using local {}. Original: {}",
                local_model.name(),
                decision.reasoning
            );

            decision.recommended_model = local_model;
            decision.estimated_cost_usd = 0.0;
            decision.should_compress = false;
        }

        self.metrics
            .write()
            .await
            .record_routing(&classification, &decision);

        // Consult spec-decoding stats. Only local models run llama.cpp, so only
        // they can benefit from (or be hurt by) NGRAM spec decoding. Cloud models
        // are unaffected and we leave their flag at the default `true` so the field
        // stays consistent if spec ever applies to remote paths in the future.
        decision.use_spec_decoding = self.decide_spec_with_event(decision.recommended_model.name());

        Ok(decision)
    }

    /// Unified routing function - routes task and returns a stream
    ///
    /// This is the main entry point for all routing operations. It:
    /// - Auto-detects task complexity and uses architect mode when beneficial
    /// - Handles quality validation and model escalation internally
    /// - Always returns a stream that can be iterated or awaited
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // Streaming (real-time UI)
    /// let stream = router.route(task, messages, Some(&tools)).await?;
    /// while let Some(chunk) = stream.next().await {
    ///     print!("{}", chunk?.content);
    /// }
    ///
    /// // Await full response
    /// let response = router.route(task, messages, None).await?.complete().await?;
    /// ```
    pub async fn route(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<RouteStream> {
        use crate::stream::{RouteResponse, RouteStream};

        // Pre-flight moderation check (before any LLM inference)
        if let Some(preflight) = &self.preflight {
            let gate_start = std::time::Instant::now();
            let moderation = preflight.check(task_description);
            arkavo_observability::subsystem_timing::global_timing()
                .dispatch_gate
                .record(gate_start.elapsed().as_millis() as u64);
            match moderation {
                preflight::ModerationResult::Allow => {}
                preflight::ModerationResult::Block {
                    policy_id, reason, ..
                } => {
                    return Err(Error::ModerationBlocked { policy_id, reason });
                }
            }
        }

        // Check complexity for architect mode
        let scorer = ComplexityScorer::new();
        let complexity = scorer.analyze(task_description);

        if complexity.architect_recommended {
            tracing::info!(
                "Architect mode activated: {} estimated subtasks",
                complexity.estimated_subtasks
            );

            let planner = ArchitectPlanner::new();
            let plan = planner.create_plan(task_description, complexity).await?;

            let executor =
                ArchitectExecutor::new(std::sync::Arc::new(self.clone_for_executor().await?));
            let arch_result = executor.execute(&plan, messages, tool_registry).await?;

            let response = RouteResponse {
                content: arch_result.final_response,
                tool_calls: Vec::new(),
                model: ModelChoice::ClaudeOpus,
                cost_usd: arch_result.actual_cost_usd,
                used_architect_mode: true,
                architect_savings: Some(arch_result.actual_savings_usd),
            };

            return Ok(RouteStream::from_response(response));
        }

        // Simple task - use Thompson Sampling routing
        let provider_response = self
            .route_with_tools(task_description, messages, tool_registry)
            .await?;

        let model = self
            .last_routed_model()
            .and_then(|name| ModelChoice::from_name(&name))
            .unwrap_or_else(|| self.selector.fastest_local_model());

        let response = RouteResponse {
            content: provider_response.content,
            tool_calls: provider_response.tool_calls,
            model,
            cost_usd: 0.0,
            used_architect_mode: false,
            architect_savings: None,
        };

        Ok(RouteStream::from_response(response))
    }

    /// Route using the fastest available local model, skipping classification.
    ///
    /// Designed for internal ML Brain tasks (episode synthesis, lesson extraction)
    /// that need speed over quality. Uses qwen3.5-0.8b or ministral-3b directly.
    pub async fn route_fast(
        &self,
        task_description: &str,
        messages: Vec<Message>,
    ) -> Result<RouteStream> {
        self.route_fast_with_preference(task_description, messages, false)
            .await
    }

    /// Like `route_fast` but prefers the largest loaded model when
    /// `prefer_capable` is true. Used for tasks like lesson synthesis
    /// where a 0.8B/3B model can't produce structured JSON reliably.
    pub async fn route_synthesis(
        &self,
        task_description: &str,
        messages: Vec<Message>,
    ) -> Result<RouteStream> {
        self.route_fast_with_preference(task_description, messages, true)
            .await
    }

    async fn route_fast_with_preference(
        &self,
        task_description: &str,
        messages: Vec<Message>,
        #[allow(unused_variables)] prefer_capable: bool,
    ) -> Result<RouteStream> {
        use crate::stream::{RouteResponse, RouteStream};

        let preferred = self.selector.fastest_local_model();
        // Prefer a model already loaded in the registry to avoid ~1s reload.
        // Synthesis tasks don't need a specific model — any loaded one will do.
        #[cfg(feature = "llama-cpp")]
        let model = if prefer_capable {
            // Pick the largest already-loaded model for tasks needing
            // structured output (lesson synthesis, pattern analysis).
            crate::ModelChoice::ALL_LOCAL
                .iter()
                .rev()
                .find(|m| self.model_registry.is_loaded(m.name()))
                .cloned()
                .unwrap_or(preferred)
        } else if self.model_registry.is_loaded(preferred.name()) {
            preferred
        } else {
            // Use any already-loaded model, sorted by preference (smallest first)
            crate::ModelChoice::ALL_LOCAL
                .iter()
                .find(|m| self.model_registry.is_loaded(m.name()))
                .cloned()
                .unwrap_or(preferred)
        };
        #[cfg(not(feature = "llama-cpp"))]
        let model = preferred;
        tracing::debug!(model = %model.name(), "Fast-path routing (internal task)");

        let use_spec = self.decide_spec_with_event(model.name());
        let provider = self
            .instantiate_provider_with_spec(&model, use_spec)
            .await?;

        let _permit = self
            .synthesis_semaphore
            .acquire()
            .await
            .map_err(|_| Error::ModelExecution("Synthesis semaphore closed".to_string()))?;

        let content = provider
            .complete(messages)
            .await
            .map_err(|e| Error::ModelExecution(format!("{task_description}: {e}")))?;

        let response = RouteResponse {
            content,
            tool_calls: Vec::new(),
            model,
            cost_usd: 0.0,
            used_architect_mode: false,
            architect_savings: None,
        };

        Ok(RouteStream::from_response(response))
    }

    /// Route a chat message using the fastest local model on a separate semaphore.
    ///
    /// Chat inference never blocks task/orchestrator work. Uses the smallest
    /// available model (0.8B/3B) which avoids <think> tag issues from larger
    /// reasoning models. Strips think blocks from the response.
    ///
    /// When a tool registry is provided, tools are passed to the LLM so it can
    /// produce structured tool calls (e.g. `get_time`) instead of hallucinating.
    ///
    /// `model_override` forces a specific model (for testing/benchmarking).
    pub async fn route_chat(
        &self,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
        model_override: Option<&ModelChoice>,
    ) -> Result<arkavo_llm::ProviderResponse> {
        let model = model_override
            .cloned()
            .unwrap_or_else(|| self.selector.fastest_local_model());
        tracing::debug!(model = %model.name(), "Chat-path routing (separate semaphore)");

        let use_spec = self.decide_spec_with_event(model.name());
        let provider = self
            .instantiate_provider_with_spec(&model, use_spec)
            .await?;

        let _permit = self
            .chat_semaphore
            .acquire()
            .await
            .map_err(|_| Error::ModelExecution("Chat semaphore closed".to_string()))?;

        // Build tool JSON from registry (same pattern as quality_gate.rs)
        let tools_json = match tool_registry {
            Some(registry) => {
                let detail_level = tool_extraction::detail_level_for_model(&model);
                let last_user_msg = messages
                    .iter()
                    .rev()
                    .find(|m| m.role == Role::User)
                    .map(|m| m.content.as_str())
                    .unwrap_or("");
                let keywords = tool_extraction::extract_keywords(last_user_msg);
                let tool_infos =
                    tool_extraction::search_tools_hybrid(registry, &keywords, detail_level, None)
                        .await;
                if tool_infos.is_empty() {
                    None
                } else {
                    Some(arkavo_llm::McpConverter::to_anthropic_format_minimal(
                        &tool_infos,
                    ))
                }
            }
            None => None,
        };

        let mut response = provider
            .complete_with_tools(messages, tools_json, None)
            .await
            .map_err(|e| Error::ModelExecution(format!("chat: {e}")))?;

        // Strip <think> blocks that small models may still emit
        response.content = crate::response::strip_think_blocks(&response.content);

        // Filter tool calls (remove language-fence false positives)
        if !response.tool_calls.is_empty() {
            response.tool_calls =
                tool_extraction::filter_and_extract_tool_calls(response.tool_calls);
        }

        // Fall back to text extraction if provider didn't parse tool calls
        if response.tool_calls.is_empty() && tool_registry.is_some() {
            let text_calls = tool_extraction::extract_tool_calls_from_text(&response.content);
            if !text_calls.is_empty() {
                response.tool_calls = text_calls;
            }
        }

        Ok(response)
    }

    /// Get the fastest available local model choice (for callers that need to know)
    pub fn fastest_local_model(&self) -> ModelChoice {
        self.selector.fastest_local_model()
    }

    /// Minimum context size across all currently-loaded local models.
    /// Returns conservative default (4096) if no models are loaded.
    /// Used by ConversationWindow to compute the history token budget.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn min_feasible_context_size(&self) -> usize {
        let models = self.model_registry.model_names();
        if models.is_empty() {
            return 4096;
        }
        let mut min_ctx = usize::MAX;
        for name in &models {
            if let Some(model) = self.model_registry.get(name) {
                let ctx = model.get_trained_context_size() as usize;
                if ctx < min_ctx {
                    min_ctx = ctx;
                }
            }
        }
        if min_ctx == usize::MAX { 4096 } else { min_ctx }
    }

    #[cfg(any(not(feature = "llama-cpp"), target_env = "musl"))]
    pub fn min_feasible_context_size(&self) -> usize {
        4096
    }

    /// Get an Arc<LlamaModel> from any loaded model for token estimation.
    /// Returns None if no models are loaded.
    #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
    pub fn any_loaded_model(&self) -> Option<std::sync::Arc<arkavo_llm::LlamaModel>> {
        let names = self.model_registry.model_names();
        names.first().and_then(|name| self.model_registry.get(name))
    }

    /// Persist validated dynamic adjustments in the background.
    ///
    /// Debounced: only flushes every 10 responses or 60 seconds, whichever
    /// comes first, to avoid chatty SQLite writes on every LLM call.
    #[cfg(feature = "advisor-persistence")]
    fn persist_advisor_state(&self) {
        use std::sync::atomic::Ordering;

        const FLUSH_EVERY_N: u64 = 10;
        const FLUSH_TIMEOUT: std::time::Duration = std::time::Duration::from_mins(1);

        let count = self.advisor_persist_count.fetch_add(1, Ordering::Relaxed) + 1;
        let timed_out = self
            .advisor_last_persist
            .lock()
            .map(|last| last.elapsed() >= FLUSH_TIMEOUT)
            .unwrap_or(false);

        if count < FLUSH_EVERY_N && !timed_out {
            return;
        }

        self.advisor_persist_count.store(0, Ordering::Relaxed);
        if let Ok(mut last) = self.advisor_last_persist.lock() {
            *last = std::time::Instant::now();
        }

        if let Some(store) = &self.advisor_store {
            let snapshots = self.advisor.export_dynamic();
            let store = store.clone();
            tokio::spawn(async move {
                let to_save: Vec<arkavo_memory::PersistedAdjustment> = snapshots
                    .iter()
                    .filter(|s| s.feedback_count >= 3 && s.success_rate >= 0.5)
                    .map(|s| arkavo_memory::PersistedAdjustment {
                        label: s.label.clone(),
                        model_family: s.model_family.clone(),
                        issue: format!("{:?}", s.issue),
                        text: s.text.clone(),
                        success_rate: s.success_rate,
                        applications: s.applications,
                        feedback_count: s.feedback_count,
                        updated_at: chrono::Utc::now(),
                    })
                    .collect();
                if !to_save.is_empty() {
                    let _ = store.save_batch(&to_save).await;
                }
            });
        }
    }

    /// Create a clone of Router for use in executor
    pub(crate) async fn clone_for_executor(&self) -> Result<Self> {
        Ok(Self {
            classifier: self.classifier.clone(),
            selector: self.selector.clone(),
            model_learning: self.model_learning.clone(),
            #[cfg(feature = "llama-cpp")]
            model_registry: self.model_registry.clone(),
            #[cfg(feature = "llama-cpp")]
            identity: self.identity.clone(),
            model_cooldowns: self.model_cooldowns.clone(),
            metrics: self.metrics.clone(),
            connectivity: self.connectivity.clone(),
            offline_mode: self.offline_mode,
            preflight: self.preflight.clone(),
            advisor: self.advisor.clone(),
            #[cfg(feature = "critic")]
            critic: self.critic.clone(),
            #[cfg(feature = "advisor-persistence")]
            advisor_store: self.advisor_store.clone(),
            #[cfg(feature = "advisor-persistence")]
            advisor_persist_count: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "advisor-persistence")]
            advisor_last_persist: std::sync::Mutex::new(std::time::Instant::now()),
            #[cfg(feature = "tdf-encrypt")]
            tdf_encryptor: self.tdf_encryptor.clone(),
            #[cfg(feature = "tdf-encrypt")]
            tdf_audit_store: self.tdf_audit_store.clone(),
            optimal_configs: self.optimal_configs.clone(),
            inference_semaphore: self.inference_semaphore.clone(),
            chat_semaphore: self.chat_semaphore.clone(),
            synthesis_semaphore: self.synthesis_semaphore.clone(),
            last_routed_model: self.last_routed_model.clone(),
            last_decision_trace: self.last_decision_trace.clone(),
            recent_traces: self.recent_traces.clone(),
            reward_failure_counts: self.reward_failure_counts.clone(),
            spec_stats: self.spec_stats.clone(),
            pending_events: self.pending_events.clone(),
            cloud_policy: self.cloud_policy,
            feasibility_baseline: self.feasibility_baseline.clone(),
            budget_tracker: self.budget_tracker.clone(),
            pricing: Arc::clone(&self.pricing),
            cloud_confirmation: std::sync::atomic::AtomicBool::new(false),
        })
    }

    pub async fn get_metrics(&self) -> RoutingMetrics {
        self.metrics.read().await.clone()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[tokio::test]
    async fn test_router_creation() {
        let result = Router::new().await;
        if result.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        assert!(result.is_ok());
    }

    #[spec("ROUTER-003")]
    #[tokio::test]
    async fn test_min_feasible_context_size_default() {
        let router = match Router::new_offline().await {
            Ok(r) => r,
            Err(_) => {
                eprintln!("Skipping test: Router::new_offline requires llama-cpp");
                return;
            }
        };
        let size = router.min_feasible_context_size();
        assert_eq!(size, 4096);
    }

    /// #635 acceptance test: the live spend-plane gate (`projected_cloud_cost`)
    /// prices a cloud arm from authored manifest rates when present, and falls
    /// back to the static per-model estimate when the arm is absent. Proves
    /// "editing a model rate in the authored config changes the cost the live
    /// budget gate uses."
    #[spec("BUDGET-010")]
    #[tokio::test]
    async fn projected_cloud_cost_uses_authored_pricing_over_static() {
        use arkavo_budget::provider_costs::{PricingEntry, ProviderPricing};

        // A decision whose first cloud fallback arm is GeminiFlash.
        let decision = RoutingDecision {
            recommended_model: ModelChoice::GeminiFlash,
            fallback_chain: vec![ModelChoice::GeminiFlash],
            confidence: 0.9,
            reasoning: String::new(),
            estimated_cost_usd: 0.0,
            estimated_time: std::time::Duration::ZERO,
            task_category: TaskCategory::FrontendUI,
            should_compress: false,
            compression_target: None,
            use_spec_decoding: false,
            trace: arkavo_router_learning_default_trace(),
        };

        // Baseline: empty pricing → static estimate.
        let router = match Router::new_offline().await {
            Ok(r) => r,
            Err(_) => {
                eprintln!("Skipping: Router::new_offline requires llama-cpp");
                return;
            }
        };
        let static_cost = router.projected_cloud_cost(&decision);

        // Authored: a deliberately distinct rate for gemini-flash-latest.
        // FrontendUI = 500 input / 2000 output. TokenCost is integer cents, so
        // use rates that clear 1 cent: $100/MTok in (10000 cents) + $50/MTok
        // out (5000 cents) → (500/1M)*10000 + (2000/1M)*5000 = 5 + 10 = 15 cents.
        // The static table for GeminiFlash gives a sub-cent result (0 cents),
        // so authored (15) is cleanly distinguishable from static (0).
        let mut pricing = ProviderPricing::new();
        pricing.register(&PricingEntry {
            model_id: "gemini-flash-latest".to_string(),
            provider: "google".to_string(),
            input_cents_per_mtok: 10000,
            output_cents_per_mtok: 5000,
            cached_input_cents_per_mtok: None,
            cache_write_cents_per_mtok: None,
            context_window: None,
            max_output_tokens: None,
        });
        let router = router.with_pricing(pricing);
        let authored_cost = router.projected_cloud_cost(&decision);

        assert_ne!(
            static_cost.as_cents(),
            authored_cost.as_cents(),
            "authored pricing must change the projected cost vs the static estimate"
        );
        assert_eq!(
            authored_cost.as_cents(),
            15,
            "authored cost should match the 15-cent rate we set (5 in + 10 out)"
        );
        assert_eq!(
            static_cost.as_cents(),
            0,
            "static estimate for GeminiFlash/FrontendUI is sub-cent → 0 cents; \
             the authored rate is what makes the gate see real cost"
        );
    }

    /// #635: `apply_manifest_pricing` updates the live gate in place on a
    /// shared `&Arc<Router>` (the path the specialization handler uses).
    #[spec("ROUTER-023")]
    #[tokio::test]
    async fn apply_manifest_pricing_updates_live_gate() {
        use arkavo_budget::provider_costs::{PricingEntry, ProviderPricing};

        let router = Arc::new(match Router::new_offline().await {
            Ok(r) => r,
            Err(_) => {
                eprintln!("Skipping: Router::new_offline requires llama-cpp");
                return;
            }
        });
        let decision = RoutingDecision {
            recommended_model: ModelChoice::GeminiFlash,
            fallback_chain: vec![ModelChoice::GeminiFlash],
            confidence: 0.9,
            reasoning: String::new(),
            estimated_cost_usd: 0.0,
            estimated_time: std::time::Duration::ZERO,
            task_category: TaskCategory::FrontendUI,
            should_compress: false,
            compression_target: None,
            use_spec_decoding: false,
            trace: arkavo_router_learning_default_trace(),
        };

        let before = router.projected_cloud_cost(&decision);

        // Apply authored pricing through the live setter (shared-reference path).
        let mut pricing = ProviderPricing::new();
        pricing.register(&PricingEntry {
            model_id: "gemini-flash-latest".to_string(),
            provider: "google".to_string(),
            input_cents_per_mtok: 10000,
            output_cents_per_mtok: 5000,
            cached_input_cents_per_mtok: None,
            cache_write_cents_per_mtok: None,
            context_window: None,
            max_output_tokens: None,
        });
        router.apply_manifest_pricing(pricing);

        let after = router.projected_cloud_cost(&decision);
        assert_ne!(
            before.as_cents(),
            after.as_cents(),
            "live pricing update must change the gate's projected cost"
        );
        assert_eq!(
            after.as_cents(),
            15,
            "after live apply the gate must use the authored 15-cent rate"
        );
    }

    /// #635: a populated registry that omits the cloud arm falls back to the
    /// static estimate with a non-zero cost, and an empty registry falls back
    /// silently (backward-compatible zero-static path). Exercises ROUTER-023
    /// from the missing-model/empty-registry angle.
    #[spec("ROUTER-023")]
    #[tokio::test]
    async fn manifest_pricing_missing_model_falls_back_to_static() {
        use arkavo_budget::provider_costs::{PricingEntry, ProviderPricing};

        let router = Arc::new(match Router::new_offline().await {
            Ok(r) => r,
            Err(_) => {
                eprintln!("Skipping: Router::new_offline requires llama-cpp");
                return;
            }
        });
        // Use a model whose static estimate is non-zero (ClaudeSonnet) but do
        // NOT register it in the authored registry. The fallback to static must
        // still produce the same value as the empty-registry baseline.
        let decision = RoutingDecision {
            recommended_model: ModelChoice::ClaudeSonnet,
            fallback_chain: vec![ModelChoice::ClaudeSonnet],
            confidence: 0.9,
            reasoning: String::new(),
            estimated_cost_usd: 0.0,
            estimated_time: std::time::Duration::ZERO,
            task_category: TaskCategory::CodeGeneration,
            should_compress: false,
            compression_target: None,
            use_spec_decoding: false,
            trace: arkavo_router_learning_default_trace(),
        };

        let empty_registry_cost = router.projected_cloud_cost(&decision);

        // Populate the registry with a different model so the arm is missing.
        let mut pricing = ProviderPricing::new();
        pricing.register(&PricingEntry {
            model_id: "gemini-flash-latest".to_string(),
            provider: "google".to_string(),
            input_cents_per_mtok: 10000,
            output_cents_per_mtok: 5000,
            cached_input_cents_per_mtok: None,
            cache_write_cents_per_mtok: None,
            context_window: None,
            max_output_tokens: None,
        });
        router.apply_manifest_pricing(pricing);

        let populated_registry_cost = router.projected_cloud_cost(&decision);
        assert_eq!(
            empty_registry_cost.as_cents(),
            populated_registry_cost.as_cents(),
            "missing model in populated registry must fall back to static estimate"
        );
        assert!(
            populated_registry_cost.as_cents() > 0,
            "ClaudeSonnet static estimate for CodeGeneration must be non-zero"
        );

        // Empty registry path: replacing with empty pricing reverts to the same static.
        router.apply_manifest_pricing(ProviderPricing::new());
        let reverted_cost = router.projected_cloud_cost(&decision);
        assert_eq!(
            reverted_cost.as_cents(),
            empty_registry_cost.as_cents(),
            "empty registry must fall back silently to static estimate"
        );
    }

    /// Helper: a minimal `DecisionTrace` for constructing test decisions.
    fn arkavo_router_learning_default_trace() -> crate::learning::DecisionTrace {
        crate::learning::DecisionTrace::thompson(
            TaskCategory::FrontendUI,
            0.0,
            vec![],
            vec![],
            "",
            0,
            0.0,
            vec![],
        )
    }
}
