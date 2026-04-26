use crate::agent_assignment::AgentAssignment;
use crate::attempt_history::AttemptHistory;
use crate::cognitive_engine_planning::Planner;
use crate::cognitive_engine_pr::PrCreator;
use crate::cognitive_engine_verification::Verifier;
use crate::error::{Error, Result};
use crate::plan_validator;
use crate::step_context::StepTrace;
use crate::token_estimator;
use arkavo_budget::BudgetTracker;
use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_github::IssueOperations;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_memory::{PlanStateStore, PlanStatus};
use arkavo_router::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// R4: per-step execution timeout. A single `do_step` call dispatches N
/// command-level LLM invocations; this bounds the total wall-clock time
/// for the step, preventing zombie work on a hung provider.
const STEP_EXECUTION_TIMEOUT: Duration = Duration::from_secs(300);

/// R4: per-command execution timeout inside a step.
const COMMAND_EXECUTION_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub id: Uuid,
    pub issue_number: u64,
    pub repository: String,
    pub steps: Vec<PlanStep>,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub step_number: usize,
    pub description: String,
    pub commands: Vec<String>,
    pub verification: Vec<VerificationCheck>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationCheck {
    TestsPassing,
    LinterClean,
    BuildSuccessful,
    FileConstraint { max_lines: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub success: bool,
    pub steps_completed: usize,
    pub total_tokens_used: u32,
    pub verification_results: Vec<VerificationResult>,
    pub final_comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub check: VerificationCheck,
    pub passed: bool,
    pub details: String,
}

pub struct CognitiveEngine {
    #[allow(dead_code)]
    budget_tracker: Arc<BudgetTracker>,
    event_writer: Arc<EventWriter>,
    github_ops: Arc<IssueOperations>,
    router: Arc<Router>,
    tool_registry: Arc<ToolRegistry>,
    session_id: String,
    sequence: std::sync::atomic::AtomicU64,
    plan_store: Option<Arc<PlanStateStore>>,
    planner: Planner,
    verifier: Verifier,
    pr_creator: PrCreator,
    /// R1: Reflexion-style failure memory across outer retries.
    attempt_history: Arc<AttemptHistory>,
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
    /// allowing callers (or tests) to share a single history across the
    /// lifetime of the orchestrator so Reflexion-style retry memory
    /// survives cognitive cycles.
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
            budget_tracker.clone(),
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
            budget_tracker,
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

        let mut total_tokens = 0u32;
        let mut steps_completed = 0usize;
        let mut verification_results = Vec::new();

        let plan = self.planner.plan(&assignment).await?;
        total_tokens += plan.estimated_tokens;

        // R5: Plan contract validation. Reject structurally malformed plans
        // before we waste any execution cycles on them.
        let validation = plan_validator::validate(&plan);
        if !validation.is_valid() {
            let summary = validation.summary();
            error!(%summary, "plan failed contract validation");
            self.log_event("plan_validation_failed", &validation.violations)
                .await;
            self.post_progress(
                &assignment,
                &format!("❌ Plan contract validation failed: {summary}"),
            )
            .await?;
            self.attempt_history.record_failure(
                &assignment.repository,
                assignment.issue_number,
                0,
                plan.steps.len(),
                &[],
                format!("plan validation failed: {summary}"),
            );
            if let Some(store) = &self.plan_store {
                let _ = store.mark_failed(plan.id, &summary).await;
            }
            return Ok(ExecutionResult {
                success: false,
                steps_completed: 0,
                total_tokens_used: total_tokens,
                verification_results: Vec::new(),
                final_comment: format!("Plan failed contract validation: {summary}"),
            });
        }

        // Update plan status to Executing
        if let Some(store) = &self.plan_store {
            if let Err(e) = store.update_status(plan.id, PlanStatus::Executing).await {
                warn!(error = %e, "Failed to update plan status to Executing");
            }
        }

        self.log_event("plan_generated", &plan).await;
        self.post_progress(&assignment, "📋 Execution plan generated")
            .await?;

        for (idx, step) in plan.steps.iter().enumerate() {
            if !self.check_budget(total_tokens, budget, &assignment).await? {
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

            match timeout(STEP_EXECUTION_TIMEOUT, self.do_step(step)).await {
                Err(_) => {
                    error!(step = idx + 1, timeout_secs = STEP_EXECUTION_TIMEOUT.as_secs(), "step timed out");
                    self.post_progress(
                        &assignment,
                        &format!(
                            "⏱️ Step {} timed out after {}s; aborting",
                            idx + 1,
                            STEP_EXECUTION_TIMEOUT.as_secs()
                        ),
                    )
                    .await?;
                    break;
                }
                Ok(Ok(tokens)) => {
                    total_tokens += tokens;
                    steps_completed += 1;

                    // Update progress in plan store
                    if let Some(store) = &self.plan_store {
                        let results_json = serde_json::to_string(&verification_results).ok();
                        if let Err(e) = store
                            .update_progress(plan.id, steps_completed, results_json.as_deref())
                            .await
                        {
                            warn!(error = %e, "Failed to update plan progress");
                        }
                    }

                    let check_results = self.verifier.check(step).await?;
                    verification_results.extend(check_results.clone());

                    if check_results.iter().any(|r| !r.passed) {
                        warn!(step = idx + 1, "Verification failed, attempting adjustment");
                        self.post_progress(
                            &assignment,
                            &format!("⚠️ Step {} verification failed, adjusting...", idx + 1),
                        )
                        .await?;

                        if let Some(adjusted_step) =
                            self.planner.adjust(step, &check_results).await?
                        {
                            match timeout(STEP_EXECUTION_TIMEOUT, self.do_step(&adjusted_step)).await {
                                Err(_) => {
                                    error!(step = idx + 1, "adjusted step timed out");
                                    break;
                                }
                                Ok(Ok(adj_tokens)) => {
                                    total_tokens += adj_tokens;
                                    let recheck_results =
                                        self.verifier.check(&adjusted_step).await?;
                                    verification_results.extend(recheck_results.clone());

                                    if recheck_results.iter().any(|r| !r.passed) {
                                        error!(step = idx + 1, "Adjustment failed verification");
                                        break;
                                    }
                                }
                                Ok(Err(e)) => {
                                    error!(step = idx + 1, error = %e, "Adjustment execution failed");
                                    break;
                                }
                            }
                        }
                    }
                }
                Ok(Err(e)) => {
                    error!(step = idx + 1, error = %e, "Step execution failed");
                    self.post_progress(&assignment, &format!("❌ Step {} failed: {}", idx + 1, e))
                        .await?;
                    break;
                }
            }
        }

        let success =
            steps_completed == plan.steps.len() && verification_results.iter().all(|r| r.passed);

        let final_comment = if success {
            format!(
                "✅ Issue completed successfully!\n\n\
                - Steps completed: {}/{}\n\
                - Tokens used: {}/{}\n\
                - All verifications passed",
                steps_completed,
                plan.steps.len(),
                total_tokens,
                budget
            )
        } else {
            format!(
                "⚠️ Issue execution incomplete\n\n\
                - Steps completed: {}/{}\n\
                - Tokens used: {}/{}\n\
                - Verification failures: {}",
                steps_completed,
                plan.steps.len(),
                total_tokens,
                budget,
                verification_results.iter().filter(|r| !r.passed).count()
            )
        };

        self.post_progress(&assignment, &final_comment).await?;

        // R1: Update attempt history. On success, clear the history so a
        // future unrelated re-run starts fresh; on failure, record the
        // outcome so the next outer retry can learn from it.
        if success {
            self.attempt_history
                .clear(&assignment.repository, assignment.issue_number);
        } else {
            self.attempt_history.record_failure(
                &assignment.repository,
                assignment.issue_number,
                steps_completed,
                plan.steps.len(),
                &verification_results,
                format!(
                    "execution incomplete: {}/{} steps, {} verification failure(s)",
                    steps_completed,
                    plan.steps.len(),
                    verification_results.iter().filter(|r| !r.passed).count()
                ),
            );
        }

        // Update final plan status
        if let Some(store) = &self.plan_store {
            if success {
                if let Err(e) = store.mark_completed(plan.id).await {
                    warn!(error = %e, "Failed to mark plan as completed");
                }
            } else if let Err(e) = store.mark_failed(plan.id, &final_comment).await {
                warn!(error = %e, "Failed to mark plan as failed");
            }
        }

        if success {
            info!("Execution successful, creating pull request");
            match self
                .pr_creator
                .create_pull_request(&assignment, &plan, steps_completed, total_tokens)
                .await
            {
                Ok(pr_url) => {
                    info!(pr_url, "Pull request created successfully");
                    let pr_comment =
                        format!("🎉 Pull request created: {pr_url}\n\n{final_comment}");
                    self.post_progress(&assignment, &pr_comment).await?;
                }
                Err(e) => {
                    warn!(error = %e, "Failed to create pull request");
                    let fallback = format!(
                        "{final_comment}\n\n⚠️ Note: Could not create PR automatically: {e}"
                    );
                    self.post_progress(&assignment, &fallback).await?;
                }
            }
        }

        Ok(ExecutionResult {
            success,
            steps_completed,
            total_tokens_used: total_tokens,
            verification_results,
            final_comment,
        })
    }

    async fn do_step(&self, step: &PlanStep) -> Result<u32> {
        debug!(step = step.step_number, "Executing step");

        let mut tokens_used = 0u32;
        // C5: Accumulate a rolling reasoning trace across the commands of
        // this step so the model retains context from one command to the
        // next, instead of re-planning from scratch each time.
        let mut trace = StepTrace::new();

        for command in &step.commands {
            debug!(command, "Executing command");

            let task_prompt = format!(
                "Execute this command as part of fixing a GitHub issue:\n\
                Step {}: {}\n\
                Command: {}\n\n\
                Use the available tools to complete this task. \
                You have access to filesystem, git, and github tools.",
                step.step_number, step.description, command
            );

            // C5: prepend the rolling trace (if any) to the prompt.
            let messages = trace.build_messages(task_prompt.clone());

            // R4: bound a single command's wall-clock time.
            let call =
                self.router
                    .route_with_tools(command, messages, Some(&self.tool_registry));
            let response = match timeout(COMMAND_EXECUTION_TIMEOUT, call).await {
                Err(_) => {
                    return Err(Error::Other(anyhow::anyhow!(
                        "command timed out after {}s: {}",
                        COMMAND_EXECUTION_TIMEOUT.as_secs(),
                        command
                    )));
                }
                Ok(Ok(r)) => r,
                Ok(Err(e)) => {
                    return Err(Error::Other(anyhow::anyhow!(
                        "Command execution failed: {e}"
                    )));
                }
            };

            info!(
                step = step.step_number,
                tool_calls = response.tool_calls.len(),
                "Command executed with {} tool calls",
                response.tool_calls.len()
            );

            // P4: use provider-reported token counts when available,
            // falling back to a more accurate heuristic than len()/4.
            let (input_tokens, output_tokens) =
                token_estimator::tokens_from_response(&task_prompt, &response);
            tokens_used = tokens_used.saturating_add(input_tokens).saturating_add(output_tokens);

            if !response.tool_calls.is_empty() {
                debug!(
                    "Tool calls executed: {:?}",
                    response
                        .tool_calls
                        .iter()
                        .map(|tc| &tc.tool_name)
                        .collect::<Vec<_>>()
                );
            }

            // C5: record this command's outcome into the rolling trace
            // so the next command in this step starts with context.
            trace.record(command, &response);
        }

        Ok(tokens_used)
    }

    async fn check_budget(
        &self,
        used: u32,
        total: u32,
        assignment: &AgentAssignment,
    ) -> Result<bool> {
        let percentage = (used as f32 / total as f32) * 100.0;

        if percentage >= 50.0 {
            let message = format!("⚠️ Budget warning: {percentage:.0}% used ({used}/{total})");
            warn!("{}", message);
            self.post_progress(assignment, &message).await?;
        }

        if percentage >= 100.0 {
            error!("Budget exhausted");
            self.post_progress(assignment, "❌ Budget exhausted, halting execution")
                .await?;
            return Ok(false);
        }

        Ok(true)
    }

    async fn post_progress(&self, assignment: &AgentAssignment, message: &str) -> Result<()> {
        let (owner, repo) = Self::parse_repository(&assignment.repository)?;

        self.github_ops
            .post_comment(&owner, &repo, assignment.issue_number, message)
            .await?;
        Ok(())
    }

    async fn log_event<T: Serialize>(&self, event_type: &str, data: &T) {
        let seq = self
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let payload = EventPayload::ReasoningStep {
            step_type: event_type.to_string(),
            description: serde_json::to_string(data).unwrap_or_default(),
            metadata: Some(serde_json::Value::Object(serde_json::Map::new())),
        };

        let event = Event::new(
            self.session_id.clone(),
            seq,
            "github-orchestrator".to_string(),
            payload,
        );

        if let Err(e) = self.event_writer.write(event).await {
            error!(error = %e, "Failed to log event");
        }
    }

    fn parse_repository(full_name: &str) -> Result<(String, String)> {
        let parts: Vec<&str> = full_name.split('/').collect();
        if parts.len() != 2 {
            return Err(Error::Other(anyhow::anyhow!(
                "Invalid repository format: {full_name}"
            )));
        }
        Ok((parts[0].to_string(), parts[1].to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repository() {
        let (owner, repo) = CognitiveEngine::parse_repository("arkavo-org/arkavo-edge").unwrap();
        assert_eq!(owner, "arkavo-org");
        assert_eq!(repo, "arkavo-edge");
    }

    #[test]
    fn test_parse_repository_invalid() {
        let result = CognitiveEngine::parse_repository("invalid");
        assert!(result.is_err());
    }
}
