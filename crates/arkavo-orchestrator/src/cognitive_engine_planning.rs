use crate::agent_assignment::AgentAssignment;
use crate::cognitive_engine_core::{
    ExecutionPlan, PlanStep, VerificationCheck, VerificationResult,
};
use crate::error::{Error, Result};
use arkavo_budget::{BudgetTracker, TokenCost, cost::TokenUsage};
use arkavo_llm::{Message as LlmMessage, Provider, Role};
use arkavo_router::Router;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub struct Planner {
    budget_tracker: Arc<BudgetTracker>,
    router: Arc<Router>,
}

impl Planner {
    pub fn new(budget_tracker: Arc<BudgetTracker>, router: Arc<Router>) -> Self {
        Self {
            budget_tracker,
            router,
        }
    }

    pub async fn plan(&self, assignment: &AgentAssignment) -> Result<ExecutionPlan> {
        debug!("Generating execution plan");

        let planning_prompt = format!(
            "Generate a detailed execution plan for this GitHub issue:\n\n\
            Title: {}\n\n\
            Description: {}\n\n\
            Type: {:?}\n\
            Complexity: {:?}\n\
            Technologies: {:?}\n\n\
            Return a structured plan with 3-5 concrete steps. For each step:\n\
            1. Brief description\n\
            2. Specific commands to execute (e.g., cargo test, cargo build, git commands)\n\
            3. Verification checks (tests, linter, build success)\n\n\
            Format each step as:\n\
            STEP N: [description]\n\
            COMMANDS: [comma-separated commands]\n\
            VERIFY: [comma-separated: tests, linter, build, or file_constraint_400]\n\
            CONFIDENCE: [0.0-1.0]",
            assignment.issue_title,
            assignment.issue_body,
            assignment.routing_decision.analysis.issue_type,
            assignment.routing_decision.analysis.complexity,
            assignment.routing_decision.analysis.technologies
        );

        let decision = self
            .router
            .classify(&planning_prompt)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Routing failed: {e}")))?;

        info!(
            model = ?decision.recommended_model,
            estimated_cost = decision.estimated_cost_usd,
            "Planning with selected model"
        );

        let planning_provider: Arc<dyn Provider> = match decision.recommended_model {
            arkavo_router::ModelChoice::LocalQwen3
            | arkavo_router::ModelChoice::LocalMinistral3B
            | arkavo_router::ModelChoice::LocalMinistral8B
            | arkavo_router::ModelChoice::LocalGemma270M
            | arkavo_router::ModelChoice::LocalGemma4B
            | arkavo_router::ModelChoice::LocalGemma12B
            | arkavo_router::ModelChoice::LocalDeepSeekCoder => {
                return Err(Error::Other(anyhow::anyhow!(
                    "Local models not yet supported for planning. Set GEMINI_API_KEY for remote planning."
                )));
            }
            _ => {
                if let Some(gemini) = self.router.get_planning_provider() {
                    Arc::new(gemini)
                } else {
                    return Err(Error::Other(anyhow::anyhow!(
                        "Planning model not available. Set GEMINI_API_KEY for remote planning."
                    )));
                }
            }
        };

        let messages = vec![LlmMessage {
            role: Role::User,
            content: planning_prompt.clone(),
            images: None,
        }];

        let response = planning_provider
            .complete(messages)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Planning LLM call failed: {e}")))?;

        let steps = self.parse_plan_from_response(&response)?;

        let estimated_input_tokens = planning_prompt.len() as u32 / 4;
        let estimated_output_tokens = response.len() as u32 / 4;
        let total_tokens = estimated_input_tokens + estimated_output_tokens;

        let model_name = match decision.recommended_model {
            arkavo_router::ModelChoice::GeminiFlash => "gemini-1.5-flash",
            arkavo_router::ModelChoice::GeminiPro => "gemini-1.5-pro",
            _ => "unknown",
        };

        let usage = TokenUsage::new(estimated_input_tokens, estimated_output_tokens);
        let cost = TokenCost::from_dollars(decision.estimated_cost_usd);

        if let Err(e) = self
            .budget_tracker
            .record_spending(
                "github-orchestrator".to_string(),
                "gemini".to_string(),
                model_name.to_string(),
                usage,
                cost,
            )
            .await
        {
            warn!(error = %e, "Failed to record budget usage for planning");
        }

        Ok(ExecutionPlan {
            issue_number: assignment.issue_number,
            repository: assignment.repository.clone(),
            steps,
            estimated_tokens: total_tokens,
        })
    }

    fn parse_plan_from_response(&self, response: &str) -> Result<Vec<PlanStep>> {
        let mut steps = Vec::new();
        let lines: Vec<&str> = response.lines().collect();
        let mut current_step = None;

        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with("STEP ") {
                if let Some(step) = current_step.take() {
                    steps.push(step);
                }
                let desc = line
                    .split_once(": ")
                    .map(|(_, d)| d)
                    .unwrap_or(line)
                    .to_string();
                current_step = Some(PlanStep {
                    step_number: steps.len() + 1,
                    description: desc,
                    commands: Vec::new(),
                    verification: Vec::new(),
                    confidence: 0.8,
                });
            } else if line.starts_with("COMMANDS:")
                && let Some(step) = current_step.as_mut()
            {
                let cmds = line
                    .strip_prefix("COMMANDS:")
                    .unwrap_or("")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                step.commands = cmds;
            } else if line.starts_with("VERIFY:")
                && let Some(step) = current_step.as_mut()
            {
                let checks_str = line.strip_prefix("VERIFY:").unwrap_or("");
                let checks: Vec<VerificationCheck> = checks_str
                    .split(',')
                    .filter_map(|s| {
                        let check = s.trim().to_lowercase();
                        if check.contains("test") {
                            Some(VerificationCheck::TestsPassing)
                        } else if check.contains("lint") {
                            Some(VerificationCheck::LinterClean)
                        } else if check.contains("build") {
                            Some(VerificationCheck::BuildSuccessful)
                        } else if check.contains("file_constraint") {
                            Some(VerificationCheck::FileConstraint { max_lines: 400 })
                        } else {
                            None
                        }
                    })
                    .collect();
                step.verification = checks;
            } else if line.starts_with("CONFIDENCE:")
                && let Some(step) = current_step.as_mut()
                && let Some(conf_str) = line.strip_prefix("CONFIDENCE:")
                && let Ok(conf) = conf_str.trim().parse::<f32>()
            {
                step.confidence = conf;
            }
        }

        if let Some(step) = current_step {
            steps.push(step);
        }

        if steps.is_empty() {
            warn!("No steps parsed from plan, using default");
            steps.push(PlanStep {
                step_number: 1,
                description: "Analyze and fix the issue".to_string(),
                commands: vec!["echo 'Analyzing issue'".to_string()],
                verification: vec![VerificationCheck::BuildSuccessful],
                confidence: 0.5,
            });
        }

        Ok(steps)
    }

    pub async fn adjust(
        &self,
        step: &PlanStep,
        failures: &[VerificationResult],
    ) -> Result<Option<PlanStep>> {
        debug!(step = step.step_number, "Generating adjustment plan");

        if failures.is_empty() {
            return Ok(None);
        }

        let failure_summary: Vec<String> = failures
            .iter()
            .filter(|r| !r.passed)
            .map(|r| format!("- {:?}: {}", r.check, r.details))
            .collect();

        if failure_summary.is_empty() {
            return Ok(None);
        }

        let adjustment_prompt = format!(
            "The following step failed verification:\n\n\
            Step {}: {}\n\
            Commands executed: {}\n\n\
            Verification failures:\n{}\n\n\
            Generate an adjusted plan to fix these failures. Provide:\n\
            1. Updated description\n\
            2. New commands to execute (comma-separated)\n\
            3. Same verification checks\n\n\
            Format:\n\
            STEP {}: [updated description]\n\
            COMMANDS: [comma-separated commands]\n\
            VERIFY: [same as before]\n\
            CONFIDENCE: [0.0-1.0]",
            step.step_number,
            step.description,
            step.commands.join(", "),
            failure_summary.join("\n"),
            step.step_number
        );

        let decision = self
            .router
            .classify(&adjustment_prompt)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Routing failed: {e}")))?;

        info!(
            model = ?decision.recommended_model,
            "Using {:?} for adjustment generation",
            decision.recommended_model
        );

        let provider: Arc<dyn Provider> = if let Some(gemini) = self.router.get_planning_provider()
        {
            Arc::new(gemini)
        } else {
            return Err(Error::Other(anyhow::anyhow!(
                "Adjustment requires Gemini. Set GEMINI_API_KEY."
            )));
        };

        let messages = vec![LlmMessage {
            role: Role::User,
            content: adjustment_prompt.clone(),
            images: None,
        }];

        let response = provider
            .complete(messages)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Adjustment LLM call failed: {e}")))?;

        let estimated_input_tokens = adjustment_prompt.len() as u32 / 4;
        let estimated_output_tokens = response.len() as u32 / 4;

        let model_name = match decision.recommended_model {
            arkavo_router::ModelChoice::GeminiFlash => "gemini-1.5-flash",
            arkavo_router::ModelChoice::GeminiPro => "gemini-1.5-pro",
            _ => "unknown",
        };

        let usage = TokenUsage::new(estimated_input_tokens, estimated_output_tokens);
        let cost = TokenCost::from_dollars(decision.estimated_cost_usd);

        if let Err(e) = self
            .budget_tracker
            .record_spending(
                "github-orchestrator".to_string(),
                "gemini".to_string(),
                model_name.to_string(),
                usage,
                cost,
            )
            .await
        {
            warn!(error = %e, "Failed to record budget usage for adjustment");
        }

        let adjusted_steps = self.parse_plan_from_response(&response)?;

        if let Some(adjusted_step) = adjusted_steps.first() {
            info!(
                step = step.step_number,
                "Generated adjustment with {} commands",
                adjusted_step.commands.len()
            );
            Ok(Some(adjusted_step.clone()))
        } else {
            warn!(step = step.step_number, "Failed to parse adjustment");
            Ok(None)
        }
    }
}
