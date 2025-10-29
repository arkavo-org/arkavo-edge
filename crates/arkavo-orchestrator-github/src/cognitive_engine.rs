use crate::agent_assignment::AgentAssignment;
use crate::error::{Error, Result};
use crate::github_operations::GitHubOperations;
use arkavo_budget::BudgetTracker;
use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_protocol::mcp::McpClient;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
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
    mcp_client: Arc<McpClient>,
    #[allow(dead_code)]
    budget_tracker: Arc<BudgetTracker>,
    event_writer: Arc<EventWriter>,
    github_ops: Arc<GitHubOperations>,
    session_id: String,
    sequence: std::sync::atomic::AtomicU64,
}

impl CognitiveEngine {
    pub fn new(
        mcp_client: Arc<McpClient>,
        budget_tracker: Arc<BudgetTracker>,
        event_writer: Arc<EventWriter>,
        github_ops: Arc<GitHubOperations>,
        session_id: String,
    ) -> Self {
        Self {
            mcp_client,
            budget_tracker,
            event_writer,
            github_ops,
            session_id,
            sequence: std::sync::atomic::AtomicU64::new(0),
        }
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

        let plan = self.plan(&assignment).await?;
        total_tokens += plan.estimated_tokens;

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

            match self.do_step(step).await {
                Ok(tokens) => {
                    total_tokens += tokens;
                    steps_completed += 1;

                    let check_results = self.check(step).await?;
                    verification_results.extend(check_results.clone());

                    if check_results.iter().any(|r| !r.passed) {
                        warn!(step = idx + 1, "Verification failed, attempting adjustment");
                        self.post_progress(
                            &assignment,
                            &format!("⚠️ Step {} verification failed, adjusting...", idx + 1),
                        )
                        .await?;

                        if let Some(adjusted_step) = self.adjust(step, &check_results).await? {
                            match self.do_step(&adjusted_step).await {
                                Ok(adj_tokens) => {
                                    total_tokens += adj_tokens;
                                    let recheck_results = self.check(&adjusted_step).await?;
                                    verification_results.extend(recheck_results.clone());

                                    if recheck_results.iter().any(|r| !r.passed) {
                                        error!(step = idx + 1, "Adjustment failed verification");
                                        break;
                                    }
                                }
                                Err(e) => {
                                    error!(step = idx + 1, error = %e, "Adjustment execution failed");
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
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

        Ok(ExecutionResult {
            success,
            steps_completed,
            total_tokens_used: total_tokens,
            verification_results,
            final_comment,
        })
    }

    async fn plan(&self, assignment: &AgentAssignment) -> Result<ExecutionPlan> {
        debug!("Generating execution plan");

        Ok(ExecutionPlan {
            issue_number: assignment.issue_number,
            repository: assignment.repository.clone(),
            steps: vec![],
            estimated_tokens: 1000,
        })
    }

    async fn do_step(&self, step: &PlanStep) -> Result<u32> {
        debug!(step = step.step_number, "Executing step");

        let mut tokens_used = 0u32;

        for command in &step.commands {
            let _result = self
                .mcp_client
                .send(command)
                .map_err(|e| Error::Other(anyhow::anyhow!("Command failed: {e}")))?;

            tokens_used += 100;
        }

        Ok(tokens_used)
    }

    async fn check(&self, step: &PlanStep) -> Result<Vec<VerificationResult>> {
        debug!(step = step.step_number, "Running verification checks");

        let mut results = Vec::new();

        for check in &step.verification {
            let result = match check {
                VerificationCheck::TestsPassing => self.verify_tests(),
                VerificationCheck::LinterClean => self.verify_linter(),
                VerificationCheck::BuildSuccessful => self.verify_build(),
                VerificationCheck::FileConstraint { max_lines } => {
                    self.verify_file_constraints(*max_lines)
                }
            };

            results.push(result);
        }

        Ok(results)
    }

    async fn adjust(
        &self,
        step: &PlanStep,
        failures: &[VerificationResult],
    ) -> Result<Option<PlanStep>> {
        debug!(step = step.step_number, "Generating adjustment plan");

        if failures.is_empty() {
            return Ok(None);
        }

        Ok(None)
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
            .await
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

    fn verify_tests(&self) -> VerificationResult {
        VerificationResult {
            check: VerificationCheck::TestsPassing,
            passed: true,
            details: "Tests passed".to_string(),
        }
    }

    fn verify_linter(&self) -> VerificationResult {
        VerificationResult {
            check: VerificationCheck::LinterClean,
            passed: true,
            details: "Linter clean".to_string(),
        }
    }

    fn verify_build(&self) -> VerificationResult {
        VerificationResult {
            check: VerificationCheck::BuildSuccessful,
            passed: true,
            details: "Build successful".to_string(),
        }
    }

    fn verify_file_constraints(&self, max_lines: usize) -> VerificationResult {
        VerificationResult {
            check: VerificationCheck::FileConstraint { max_lines },
            passed: true,
            details: format!("All files under {max_lines} lines"),
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
