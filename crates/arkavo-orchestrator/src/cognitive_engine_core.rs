use crate::agent_assignment::AgentAssignment;
use crate::attempt_history::{AttemptHistory, FailureKind};
use crate::cognitive_engine_outcome::ExecutionOutcome;
use crate::cognitive_engine_planning::Planner;
use crate::cognitive_engine_pr::PrCreator;
use crate::cognitive_engine_step::do_step;
use crate::cognitive_engine_timing::{command_execution_timeout, step_execution_timeout};
use crate::cognitive_engine_verification::Verifier;
use crate::error::Result;
use crate::plan_validator;
use arkavo_budget::BudgetTracker;
use arkavo_events::EventWriter;
use arkavo_github::IssueOperations;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_memory::{PlanStateStore, PlanStatus};
use arkavo_router::Router;
use std::sync::Arc;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

pub use crate::cognitive_engine_types::{
    ExecutionPlan, ExecutionResult, PlanStep, VerificationCheck, VerificationResult,
};

pub struct CognitiveEngine {
    pub(crate) event_writer: Arc<EventWriter>,
    pub(crate) github_ops: Arc<IssueOperations>,
    pub(crate) router: Arc<Router>,
    pub(crate) tool_registry: Arc<ToolRegistry>,
    pub(crate) session_id: String,
    pub(crate) sequence: std::sync::atomic::AtomicU64,
    pub(crate) plan_store: Option<Arc<PlanStateStore>>,
    pub(crate) planner: Planner,
    pub(crate) verifier: Verifier,
    pub(crate) pr_creator: PrCreator,
    /// R1: Reflexion-style failure memory across outer retries.
    pub(crate) attempt_history: Arc<AttemptHistory>,
}

impl CognitiveEngine {
    pub fn new(
        budget_tracker: Arc<BudgetTracker>,
        event_writer: Arc<EventWriter>,
        github_ops: Arc<IssueOperations>,
        router: Arc<Router>,
        tool_registry: Arc<ToolRegistry>,
        session_id: String,
        plan_store: Option<Arc<PlanStateStore>>,
    ) -> Self {
        Self::new_with_attempt_history(
            budget_tracker,
            event_writer,
            github_ops,
            router,
            tool_registry,
            session_id,
            plan_store,
            Arc::new(AttemptHistory::new()),
        )
    }

    /// Construct the engine with an externally-supplied `AttemptHistory`,
    /// so Reflexion-style retry memory survives multiple cognitive cycles.
    // Every argument is a distinct, non-substitutable dependency. A
    // Builder would add friction without reducing coupling.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_attempt_history(
        budget_tracker: Arc<BudgetTracker>,
        event_writer: Arc<EventWriter>,
        github_ops: Arc<IssueOperations>,
        router: Arc<Router>,
        tool_registry: Arc<ToolRegistry>,
        session_id: String,
        plan_store: Option<Arc<PlanStateStore>>,
        attempt_history: Arc<AttemptHistory>,
    ) -> Self {
        let planner = Planner::new_with_history(
            budget_tracker,
            router.clone(),
            plan_store.clone(),
            attempt_history.clone(),
        );
        let verifier = Verifier::new();
        let pr_creator = PrCreator::new(
            tool_registry.clone(),
            event_writer.clone(),
            session_id.clone(),
        );

        Self {
            event_writer,
            github_ops,
            router,
            tool_registry,
            session_id,
            sequence: std::sync::atomic::AtomicU64::new(0),
            plan_store,
            planner,
            verifier,
            pr_creator,
            attempt_history,
        }
    }

    /// Access the shared attempt history (for external clear/query).
    pub fn attempt_history(&self) -> Arc<AttemptHistory> {
        Arc::clone(&self.attempt_history)
    }

    pub async fn execute(&self, assignment: AgentAssignment) -> Result<ExecutionResult> {
        info!(
            issue = assignment.issue_number,
            strategy = ?assignment.routing_decision.strategy,
            "Starting cognitive execution cycle"
        );

        let budget = assignment
            .routing_decision
            .analysis
            .complexity
            .token_budget();

        // Read configurable timeouts once per execution; the planner runs
        // first so a drifting env var doesn't change the policy mid-run.
        let step_timeout = step_execution_timeout();
        let command_timeout = command_execution_timeout();
        debug!(
            step_timeout_secs = step_timeout.as_secs(),
            command_timeout_secs = command_timeout.as_secs(),
            "cognitive engine execution configured"
        );

        let mut outcome = ExecutionOutcome::new();

        let plan = self.planner.plan(&assignment).await?;
        outcome.total_tokens += plan.estimated_tokens;

        // R5: Plan contract validation. Reject structurally malformed plans
        // before we waste any execution cycles on them.
        let validation = plan_validator::validate(&plan);
        if !validation.is_valid() {
            return self
                .handle_plan_invalid(&assignment, &plan, &validation, outcome)
                .await;
        }

        if let Some(store) = &self.plan_store {
            if let Err(e) = store.update_status(plan.id, PlanStatus::Executing).await {
                warn!(error = %e, "Failed to update plan status to Executing");
            }
        }

        self.log_event("plan_generated", &plan).await;
        self.post_progress(&assignment, "📋 Execution plan generated")
            .await?;

        for (idx, step) in plan.steps.iter().enumerate() {
            if !self
                .check_budget(outcome.total_tokens, budget, &assignment)
                .await?
            {
                outcome.abort_kind = Some(FailureKind::BudgetExceeded);
                outcome.abort_detail = Some(format!(
                    "budget exhausted at step {}/{} ({}/{} tokens)",
                    idx + 1,
                    plan.steps.len(),
                    outcome.total_tokens,
                    budget
                ));
                break;
            }

            info!(step = idx + 1, "Executing plan step");
            self.post_progress(
                &assignment,
                &format!(
                    "⚙️ Step {}/{}: {}",
                    idx + 1,
                    plan.steps.len(),
                    step.description
                ),
            )
            .await?;

            let tokens = match timeout(
                step_timeout,
                do_step(&self.router, &self.tool_registry, step, command_timeout),
            )
            .await
            {
                Err(_) => {
                    error!(
                        step = idx + 1,
                        timeout_secs = step_timeout.as_secs(),
                        "step timed out"
                    );
                    self.post_progress(
                        &assignment,
                        &format!("⏱️ Step {} timed out; aborting", idx + 1),
                    )
                    .await?;
                    outcome.abort_kind = Some(FailureKind::Timeout);
                    outcome.abort_detail = Some(format!(
                        "step {} timed out after {}s",
                        idx + 1,
                        step_timeout.as_secs()
                    ));
                    break;
                }
                Ok(Err(e)) => {
                    error!(step = idx + 1, error = %e, "Step execution failed");
                    self.post_progress(&assignment, &format!("❌ Step {} failed: {}", idx + 1, e))
                        .await?;
                    outcome.abort_kind = Some(FailureKind::CommandError);
                    outcome.abort_detail = Some(format!("step {} failed: {}", idx + 1, e));
                    break;
                }
                Ok(Ok(t)) => t,
            };

            outcome.total_tokens += tokens;
            outcome.steps_completed += 1;

            if let Some(store) = &self.plan_store {
                let json = serde_json::to_string(&outcome.verification_results).ok();
                if let Err(e) = store
                    .update_progress(plan.id, outcome.steps_completed, json.as_deref())
                    .await
                {
                    warn!(error = %e, "Failed to update plan progress");
                }
            }

            let check_results = self.verifier.check(step).await?;
            outcome.verification_results.extend(check_results.clone());

            if check_results.iter().any(|r| !r.passed)
                && !self
                    .run_adjustment(
                        idx,
                        step,
                        &check_results,
                        step_timeout,
                        command_timeout,
                        &assignment,
                        &mut outcome,
                    )
                    .await?
            {
                break;
            }
        }

        let success = outcome.is_success(plan.steps.len());
        let final_comment = outcome.final_comment(plan.steps.len(), budget);
        self.post_progress(&assignment, &final_comment).await?;

        // R1: record outcome (clear on success, persist failure detail otherwise).
        outcome.record_into_history(
            &self.attempt_history,
            &assignment.repository,
            assignment.issue_number,
            plan.steps.len(),
        );

        if let Some(store) = &self.plan_store {
            let res = if success {
                store.mark_completed(plan.id).await
            } else {
                store.mark_failed(plan.id, &final_comment).await
            };
            if let Err(e) = res {
                warn!(error = %e, success, "Failed to update final plan status");
            }
        }

        if success {
            self.create_pull_request_for(&assignment, &plan, &outcome, &final_comment)
                .await?;
        }

        Ok(ExecutionResult {
            success,
            steps_completed: outcome.steps_completed,
            total_tokens_used: outcome.total_tokens,
            verification_results: outcome.verification_results,
            final_comment,
        })
    }

    /// Single-shot adjustment retry when verification fails. Returns
    /// Ok(true) to continue the outer loop, Ok(false) to break with the
    /// abort cause already recorded on `outcome`.
    #[allow(clippy::too_many_arguments)]
    async fn run_adjustment(
        &self,
        idx: usize,
        step: &PlanStep,
        check_results: &[VerificationResult],
        step_timeout: std::time::Duration,
        command_timeout: std::time::Duration,
        assignment: &AgentAssignment,
        outcome: &mut ExecutionOutcome,
    ) -> Result<bool> {
        warn!(step = idx + 1, "Verification failed, attempting adjustment");
        self.post_progress(
            assignment,
            &format!("⚠️ Step {} verification failed, adjusting...", idx + 1),
        )
        .await?;

        let Some(adjusted) = self.planner.adjust(step, check_results).await? else {
            return Ok(true);
        };

        match timeout(
            step_timeout,
            do_step(
                &self.router,
                &self.tool_registry,
                &adjusted,
                command_timeout,
            ),
        )
        .await
        {
            Err(_) => {
                error!(
                    step = idx + 1,
                    timeout_secs = step_timeout.as_secs(),
                    "adjusted step timed out"
                );
                outcome.abort_kind = Some(FailureKind::Timeout);
                outcome.abort_detail = Some(format!(
                    "adjusted step {} timed out after {}s",
                    idx + 1,
                    step_timeout.as_secs()
                ));
                Ok(false)
            }
            Ok(Err(e)) => {
                error!(step = idx + 1, error = %e, "Adjustment execution failed");
                outcome.abort_kind = Some(FailureKind::CommandError);
                outcome.abort_detail = Some(format!("adjusted step {} failed: {}", idx + 1, e));
                Ok(false)
            }
            Ok(Ok(adj_tokens)) => {
                outcome.total_tokens += adj_tokens;
                let recheck = self.verifier.check(&adjusted).await?;
                outcome.verification_results.extend(recheck.clone());
                if recheck.iter().any(|r| !r.passed) {
                    error!(step = idx + 1, "Adjustment failed verification");
                    Ok(false)
                } else {
                    Ok(true)
                }
            }
        }
    }
}
