#![cfg(unix)]
#![allow(
    clippy::disallowed_methods,
    reason = "Tokio test macros create a runtime from a synchronous test entry point"
)]

use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};

use arkavo_budget::{BudgetConfig, BudgetTracker, CloudPolicy, PricingEntry, TokenCost};
use arkavo_mcp_codex::{CodexConfig, CodexWorker, RunStatus, Sandbox, SpendApproval, Usage};
use tempfile::TempDir;

/// Created before the prompt is read, so a test can prove that a descendant
/// spawned at the earliest possible moment is still inside the killed tree.
/// Its delay, and the wait each test makes before looking for it, leave room
/// for a slow machine to deliver the kill without turning that into a failure.
const EARLY_CHILD: &str = "escaped_early";

struct Fixture {
    root: TempDir,
    thread: String,
    config: CodexConfig,
    approval: SpendApproval,
    budget: Arc<BudgetTracker>,
}

impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        // A distinct thread per fixture: the worker takes a machine-wide lock
        // on the Codex thread, so tests must not claim a shared identifier.
        let thread = uuid::Uuid::new_v4().to_string();
        let executable = root.path().join("protocol-fixture");
        std::fs::write(
            &executable,
            r#"#!/bin/sh
(sleep 1.5; touch escaped_early) >/dev/null 2>&1 &
printf '%s\n' "$@" > arguments.txt
prompt=$(cat)
printf '%s' "$prompt" > prompt.txt
printf '%s\n' '{"type":"thread.started","thread_id":"THREAD_UUID"}'
case "$prompt" in
  wait) exec sleep 20 ;;
  tree) (sleep 1.5; touch escaped) >/dev/null 2>&1 & touch ready; exec sleep 20 ;;
  malformed) printf '%s\n' 'not json'; exit 0 ;;
  failed) printf '%s\n' '{"type":"turn.failed","error":{"message":"credential must not be returned"}}'; exit 0 ;;
  early) exit 0 ;;
  huge) printf '%10000s' 'x'; exit 0 ;;
esac
printf '%s\n' '{"type":"item.completed","item":{"id":"item_0","type":"file_change","status":"completed","changes":[{"path":"src/lib.rs","kind":"update"}]}}'
printf '%s\n' '{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"Finished task"}}'
printf '%s\n' '{"type":"turn.completed","usage":{"input_tokens":1000,"cached_input_tokens":400,"cache_write_input_tokens":200,"output_tokens":100,"reasoning_output_tokens":20}}'
if [ "$prompt" = exit_error ]; then exit 1; fi
"#
            .replace("THREAD_UUID", &thread),
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = CodexConfig {
            executable,
            workspace,
            agent_id: "coder".into(),
            model: "gpt-6-astra".into(),
            sandbox: Sandbox::ReadOnly,
            timeout: Duration::from_secs(5),
            max_output_bytes: 8192,
        };
        let approval = SpendApproval {
            policy: CloudPolicy::CloudWithinCap,
            user_confirmed: false,
            projected_cost: TokenCost::from_cents(1),
            pricing: PricingEntry {
                model_id: config.model.clone(),
                provider: "openai".into(),
                // Four distinct total rates, so a bucket charged at the wrong
                // one — or charged twice — changes the expected dollar figure.
                input_cents_per_mtok: 1000,
                output_cents_per_mtok: 5000,
                cached_input_cents_per_mtok: Some(100),
                cache_write_cents_per_mtok: Some(1250),
                context_window: None,
                max_output_tokens: None,
            },
        };
        let budget = Arc::new(BudgetTracker::new(BudgetConfig::default()).await.unwrap());
        Self {
            root,
            thread,
            config,
            approval,
            budget,
        }
    }

    fn open(&self) -> anyhow::Result<CodexWorker> {
        self.open_at(&self.root.path().join("session.json"))
    }

    fn open_at(&self, state: &std::path::Path) -> anyhow::Result<CodexWorker> {
        CodexWorker::open(
            self.config.clone(),
            state,
            self.approval.clone(),
            self.budget.clone(),
        )
    }

    fn escaped(&self, name: &str) -> bool {
        self.config.workspace.join(name).exists()
    }
}

#[tokio::test]
async fn starts_resumes_and_accounts_without_double_counting() {
    let f = Fixture::new().await;
    let worker = f.open().unwrap();
    let prompt = "--dangerously-bypass-approvals-and-sandbox $(touch unsafe)\ncode task";
    let result = worker.run(prompt).await.unwrap();
    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(result.message, "Finished task");
    assert_eq!(result.changes[0].path, "src/lib.rs");
    // Codex reports input_tokens=1000 with 400 of them cached and 200 of them
    // cache writes, so the disjoint buckets are 400 ordinary + 400 cached +
    // 200 written, each at its own total rate, plus 100 output:
    // (400*1000 + 400*100 + 200*1250 + 100*5000) / 1e8 = 0.0119.
    // Charging the writes as a surcharge on top of the input rate would give
    // 0.0139 instead.
    assert!((result.estimated_cost_usd.unwrap() - 0.0119).abs() < 1e-9);
    assert!(!result.accounting_incomplete);
    assert_eq!(
        std::fs::read_to_string(f.config.workspace.join("prompt.txt")).unwrap(),
        prompt
    );
    let history = f.budget.get_spending_history(10).await;
    assert_eq!(history.len(), 1);
    // Cached and freshly written tokens are attributed once each, so the total
    // still matches the input Codex reported.
    assert_eq!(history[0].usage.total_tokens(), 1100);
    assert_eq!(history[0].usage.input_tokens, 400);
    assert_eq!(history[0].usage.cached_input_tokens, 400);
    assert_eq!(history[0].usage.cache_write_tokens, 200);
    assert_eq!(history[0].usage.thinking_tokens, 20);
    assert_eq!(history[0].agent_id, "coder");
    assert_eq!(history[0].provider, "openai");
    drop(worker);
    let worker = f.open().unwrap();
    worker.run("continue").await.unwrap();
    let arguments = std::fs::read_to_string(f.config.workspace.join("arguments.txt")).unwrap();
    // `resume` is a subcommand, so it must precede the options it applies to.
    assert!(arguments.starts_with(&format!("exec\nresume\n{}\n--json\n", f.thread)));
    assert!(arguments.contains("sandbox_mode=\"read-only\""));
    assert!(arguments.ends_with("-\n"));
    assert!(!arguments.contains(prompt));
    assert_eq!(f.budget.get_spending_history(10).await.len(), 2);
}

#[tokio::test]
async fn failures_keep_session_and_require_reconciliation() {
    for prompt in ["malformed", "failed", "early", "huge"] {
        let f = Fixture::new().await;
        let worker = f.open().unwrap();
        let result = worker.run(prompt).await.unwrap();
        assert_eq!(result.status, RunStatus::Failed, "{prompt}");
        assert!(result.thread_id.is_some());
        assert!(result.accounting_incomplete);
        assert!(!result.error.unwrap().contains("credential"));
        assert!(worker.run("retry").await.is_err());
        assert!(worker.reconcile(Usage::default(), f64::NAN).await.is_err());
        worker.reconcile(Usage::default(), 0.2).await.unwrap();
        assert_eq!(f.budget.get_status().await.total_spent.as_cents(), 20);
        assert_eq!(
            worker.run("retry").await.unwrap().status,
            RunStatus::Completed
        );
        assert!(worker.reconcile(Usage::default(), 0.0).await.is_err());
    }
}

/// The `arkavo codex` recovery path. An interrupted attempt marks the state
/// file, a *later process* reopens that file, and the refusal must survive the
/// reopen — otherwise a restart would launder an unrecorded charge. Only once
/// the charge is acknowledged does the session accept work again.
#[tokio::test]
async fn a_reopened_session_runs_again_only_after_the_charge_is_acknowledged() {
    let f = Fixture::new().await;
    let interrupted = f.open().unwrap();
    assert!(
        interrupted
            .run("malformed")
            .await
            .unwrap()
            .accounting_incomplete
    );
    drop(interrupted);

    let reopened = f.open().unwrap();
    assert!(reopened.session().unwrap().accounting_incomplete);
    assert!(reopened.run("retry").await.is_err());

    reopened.reconcile(Usage::default(), 0.2).await.unwrap();
    assert!(!reopened.session().unwrap().accounting_incomplete);
    assert_eq!(f.budget.get_status().await.total_spent.as_cents(), 20);
    assert_eq!(
        reopened.run("retry").await.unwrap().status,
        RunStatus::Completed
    );
}

#[tokio::test]
async fn cache_write_tokens_are_priced_at_their_own_total_rate() {
    // The ledger this crate shares treats every per-MTok figure as the total
    // for its bucket, so an unpriced cache write falls back to the input rate
    // rather than to nothing, and is never added on top of it.
    for (rate, expected) in [
        // 940_000 cents-per-MTok-units of ordinary input, cached input and
        // output are common to every row; only the 200 written tokens move.
        (Some(1250), 0.0119),
        (Some(100), 0.0096),
        (None, 0.0114),
        (Some(0), 0.0094),
    ] {
        let mut f = Fixture::new().await;
        f.approval.pricing.cache_write_cents_per_mtok = rate;
        let result = f.open().unwrap().run("code task").await.unwrap();
        let actual = result.estimated_cost_usd.unwrap();
        assert!(
            (actual - expected).abs() < 1e-9,
            "rate {rate:?}: expected {expected}, got {actual}"
        );
        // Whatever the price, the tokens are counted exactly once.
        let history = f.budget.get_spending_history(10).await;
        assert_eq!(history[0].usage.total_tokens(), 1100);
    }
}

#[tokio::test]
async fn nonzero_exit_still_records_completed_usage() {
    let f = Fixture::new().await;
    let result = f.open().unwrap().run("exit_error").await.unwrap();
    assert_eq!(result.status, RunStatus::Failed);
    assert!(!result.accounting_incomplete);
    assert_eq!(f.budget.get_spending_history(10).await.len(), 1);
}

#[tokio::test]
async fn cancelling_idle_stdout_does_not_wait_for_run_lock() {
    let f = Fixture::new().await;
    let worker = Arc::new(f.open().unwrap());
    let runner = worker.clone();
    let task = tokio::spawn(async move { runner.run("wait").await.unwrap() });
    tokio::time::timeout(Duration::from_secs(3), async {
        while worker.session().unwrap().thread_id.is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(worker.run("concurrent").await.is_err());
    worker.cancel();
    let result = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.status, RunStatus::Cancelled);
    assert!(worker.session().unwrap().accounting_incomplete);
    tokio::time::sleep(Duration::from_millis(2000)).await;
    assert!(!f.escaped(EARLY_CHILD), "cancel left a descendant running");
}

#[tokio::test]
async fn timeout_preserves_unknown_spend_and_kills_the_tree() {
    let mut f = Fixture::new().await;
    f.config.timeout = Duration::from_millis(100);
    let worker = f.open().unwrap();
    assert_eq!(
        worker.run("wait").await.unwrap().status,
        RunStatus::TimedOut
    );
    tokio::time::sleep(Duration::from_millis(2000)).await;
    // That descendant was created before Codex read the prompt, so this also
    // proves the group covers the whole lifetime of the child.
    assert!(!f.escaped(EARLY_CHILD), "timeout left a descendant running");
    drop(worker);
    assert!(f.open().unwrap().session().unwrap().accounting_incomplete);
}

#[tokio::test]
async fn dropping_run_future_kills_descendants_and_preserves_recovery() {
    let f = Fixture::new().await;
    let worker = Arc::new(f.open().unwrap());
    let runner = worker.clone();
    let task = tokio::spawn(async move { runner.run("tree").await });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !f.config.workspace.join("ready").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::sleep(Duration::from_millis(2000)).await;
    assert!(!f.escaped("escaped"));
    assert!(!f.escaped(EARLY_CHILD));
    assert!(worker.session().unwrap().accounting_incomplete);
    assert!(worker.run("cannot silently retry").await.is_err());
}

#[tokio::test]
async fn spawn_failure_is_not_charged_and_bad_requests_never_spawn() {
    let f = Fixture::new().await;
    let worker = f.open().unwrap();
    assert!(worker.run("   ").await.is_err());
    assert!(worker.run(&"x".repeat(1024 * 1024 + 1)).await.is_err());
    std::fs::remove_file(&f.config.executable).unwrap();
    assert!(worker.run("unavailable").await.is_err());
    assert!(!worker.session().unwrap().accounting_incomplete);
    assert!(f.budget.get_spending_history(1).await.is_empty());
}

#[tokio::test]
async fn spawn_failure_does_not_burn_the_one_shot_confirmation() {
    let mut f = Fixture::new().await;
    f.approval.policy = CloudPolicy::AskBeforeCloud;
    f.approval.user_confirmed = true;
    let worker = f.open().unwrap();
    let program = std::fs::read(&f.config.executable).unwrap();
    std::fs::remove_file(&f.config.executable).unwrap();
    assert!(worker.run("unavailable").await.is_err());
    std::fs::write(&f.config.executable, program).unwrap();
    std::fs::set_permissions(&f.config.executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        worker
            .run("the confirmation survived")
            .await
            .unwrap()
            .status,
        RunStatus::Completed
    );
    assert!(worker.run("but only for one attempt").await.is_err());
}

#[tokio::test]
async fn policy_budget_and_one_shot_confirmation_are_enforced() {
    let mut f = Fixture::new().await;
    for policy in [CloudPolicy::LocalOnly, CloudPolicy::AskBeforeCloud] {
        f.approval.policy = policy;
        assert!(f.open().unwrap().run("denied").await.is_err());
        assert!(!f.config.workspace.join("arguments.txt").exists());
    }
    f.approval.user_confirmed = true;
    let worker = f.open().unwrap();
    assert_eq!(
        worker.run("allowed once").await.unwrap().status,
        RunStatus::Completed
    );
    assert!(worker.run("second requires confirmation").await.is_err());
    drop(worker);
    f.approval.policy = CloudPolicy::CloudWithinCap;
    f.approval.projected_cost = TokenCost::from_cents(10000);
    assert!(f.open().unwrap().run("over budget").await.is_err());
}

#[tokio::test]
async fn session_lock_and_binding_prevent_cross_role_resume() {
    let mut f = Fixture::new().await;
    let worker = f.open().unwrap();
    assert!(f.open().is_err());
    drop(worker);
    let saved = f.config.clone();
    f.config.agent_id = "another-role".into();
    assert!(f.open().is_err());
    f.config = saved.clone();
    f.config.sandbox = Sandbox::WorkspaceWrite;
    assert!(f.open().is_err());
    f.config = saved;
    assert!(
        CodexWorker::open(
            f.config.clone(),
            &f.config.workspace.join("state.json"),
            f.approval.clone(),
            f.budget.clone()
        )
        .is_err()
    );
    f.approval.pricing.model_id = "unknown".into();
    assert!(f.open().is_err());
}

#[tokio::test]
async fn copied_state_files_cannot_drive_one_codex_thread() {
    let f = Fixture::new().await;
    let state = f.root.path().join("session.json");
    let worker = f.open().unwrap();
    worker.run("code task").await.unwrap();
    assert_eq!(
        worker.session().unwrap().thread_id.as_deref(),
        Some(&*f.thread)
    );
    drop(worker);

    let elsewhere = f.root.path().join("copy");
    std::fs::create_dir(&elsewhere).unwrap();
    let copy = elsewhere.join("session.json");
    std::fs::copy(&state, &copy).unwrap();

    let owner = f.open().unwrap();
    // Two paths naming one Codex thread is exactly what a per-path lock cannot
    // see, and it would let two workers drive the same remote session.
    assert!(f.open_at(&copy).is_err());

    // A copy rebound to another thread is a different session and may run
    // beside the first.
    let other = uuid::Uuid::new_v4().to_string();
    let rebound = std::fs::read_to_string(&copy)
        .unwrap()
        .replace(&f.thread, &other);
    std::fs::write(&copy, rebound).unwrap();
    let neighbour = f.open_at(&copy).unwrap();
    assert_eq!(
        neighbour.session().unwrap().thread_id.as_deref(),
        Some(&*other)
    );

    drop(neighbour);
    drop(owner);
    // Releasing the owner releases its claim on the thread as well.
    f.open_at(&copy).unwrap();
}
