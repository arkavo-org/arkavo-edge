use crate::agent_assignment::AgentAssignment;
use crate::attempt_history::AttemptHistory;
use crate::cognitive_engine_core::{ExecutionPlan, PlanStep, VerificationResult};
use crate::cognitive_engine_planning_parser::{parse_plan_from_response, parse_plan_json_or_text};
use crate::cognitive_engine_schema::JsonExecutionPlan;
use crate::error::{Error, Result};
use crate::planner_config::get_planner_config;
use arkavo_budget::BudgetTracker;
use arkavo_llm::Message as LlmMessage;
use arkavo_memory::{PersistedPlan, PlanStateStore, PlanStatus};
use arkavo_router::Router;
use arkavo_router::usage::{CallBudget, estimate_request};
use chrono::Utc;
use std::sync::Arc;
use tracing::{debug, info, warn};
use uuid::Uuid;

pub struct Planner {
    budget_tracker: Arc<BudgetTracker>,
    router: Arc<Router>,
    plan_store: Option<Arc<PlanStateStore>>,
    /// R1: Reflexion-style attempt history; consulted at plan time so the
    /// model is aware of prior failures on the same issue.
    attempt_history: Arc<AttemptHistory>,
}

impl Planner {
    pub fn new(
        budget_tracker: Arc<BudgetTracker>,
        router: Arc<Router>,
        plan_store: Option<Arc<PlanStateStore>>,
    ) -> Self {
        Self::new_with_history(
            budget_tracker,
            router,
            plan_store,
            Arc::new(AttemptHistory::new()),
        )
    }

    pub fn new_with_history(
        budget_tracker: Arc<BudgetTracker>,
        router: Arc<Router>,
        plan_store: Option<Arc<PlanStateStore>>,
        attempt_history: Arc<AttemptHistory>,
    ) -> Self {
        Self {
            budget_tracker,
            router,
            plan_store,
            attempt_history,
        }
    }

    pub async fn plan(&self, assignment: &AgentAssignment) -> Result<ExecutionPlan> {
        debug!("Generating execution plan");

        // Use a simple prompt for routing to get the model decision
        let routing_prompt = format!(
            "Planning task for: {} - {:?}",
            assignment.issue_title, assignment.routing_decision.analysis.issue_type
        );

        let decision = self
            .router
            .classify(&routing_prompt)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Routing failed: {e}")))?;

        // Get capability-appropriate planner config for adaptive prompting
        let planner_config = get_planner_config(decision.recommended_model.capability());
        let base_prompt = planner_config.planning_prompt(assignment);

        // R1: If prior attempts on this issue failed, prepend a summary of
        // those failures so the model can avoid repeating the same
        // mistakes (Reflexion-style failure memory).
        let planning_prompt = match self
            .attempt_history
            .to_prompt_block(&assignment.repository, assignment.issue_number)
        {
            Some(history_block) => format!("{history_block}\n\n{base_prompt}"),
            None => base_prompt,
        };

        info!(
            model = ?decision.recommended_model,
            tier = ?planner_config.tier(),
            estimated_cost = decision.estimated_cost_usd,
            "Planning with selected model"
        );

        let (planning_provider, actual_model) = self
            .router
            .get_provider_attributed(&decision.recommended_model)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Planning provider unavailable: {e}")))?;
        let messages = vec![LlmMessage::user(planning_prompt.clone())];

        // Use structured output with JSON schema if provider supports it
        let schema = if planning_provider.supports_structured_output() {
            Some(JsonExecutionPlan::json_schema())
        } else {
            None
        };

        let estimated = estimate_request(
            &messages,
            schema.as_ref(),
            planner_config.max_tokens().unwrap_or(4096) as u32,
        );
        let budget = CallBudget {
            tracker: &self.budget_tracker,
            agent_id: "github-orchestrator",
        };
        budget
            .check(self.router.usage_cost(&actual_model, &estimated))
            .await
            .map_err(|e| Error::Other(e.into()))?;
        let response = planning_provider
            .complete_with_schema_response(messages, schema, planner_config.max_tokens())
            .await;
        let response = self
            .account_failure(response, &actual_model, &estimated, budget)
            .await?;

        // Record the paid call before parsing: malformed plans still consumed tokens.
        let usage = self
            .router
            .attribute_response(actual_model, &estimated, &response);
        budget
            .record(&usage)
            .await
            .map_err(|e| Error::Other(e.into()))?;
        let total_tokens = usage.usage.total_tokens();
        let steps = parse_plan_json_or_text(&response.content)?;

        let plan_id = Uuid::new_v4();
        let plan = ExecutionPlan {
            id: plan_id,
            issue_number: assignment.issue_number,
            repository: assignment.repository.clone(),
            steps,
            estimated_tokens: total_tokens,
        };

        // Persist the plan if store is available
        if let Some(store) = &self.plan_store {
            let persisted = PersistedPlan {
                id: plan_id,
                original_prompt: assignment.issue_body.clone(),
                plan_json: serde_json::to_string(&plan).unwrap_or_default(),
                status: PlanStatus::Planning,
                current_subtask: 0,
                total_subtasks: plan.steps.len(),
                completed_results_json: None,
                error_message: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            if let Err(e) = store.save_plan(&persisted).await {
                warn!(error = %e, "Failed to persist plan to store");
            } else {
                debug!(plan_id = %plan_id, "Plan persisted to store");
            }
        }

        Ok(plan)
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

        let (provider, actual_model) = self
            .router
            .get_provider_attributed(&decision.recommended_model)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Adjustment provider unavailable: {e}")))?;

        let messages = vec![LlmMessage::user(adjustment_prompt.clone())];

        let estimated = estimate_request(&messages, None, 4096);
        let budget = CallBudget {
            tracker: &self.budget_tracker,
            agent_id: "github-orchestrator",
        };
        budget
            .check(self.router.usage_cost(&actual_model, &estimated))
            .await
            .map_err(|e| Error::Other(e.into()))?;
        let response = provider
            .complete_with_schema_response(messages, None, None)
            .await;
        let response = self
            .account_failure(response, &actual_model, &estimated, budget)
            .await?;

        let usage = self
            .router
            .attribute_response(actual_model, &estimated, &response);
        budget
            .record(&usage)
            .await
            .map_err(|e| Error::Other(e.into()))?;
        let adjusted_steps = parse_plan_from_response(&response.content)?;

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
    async fn account_failure(
        &self,
        result: arkavo_llm::Result<arkavo_llm::ProviderResponse>,
        model: &arkavo_router::ModelChoice,
        estimated: &arkavo_budget::cost::TokenUsage,
        budget: CallBudget<'_>,
    ) -> Result<arkavo_llm::ProviderResponse> {
        match result {
            Ok(response) => Ok(response),
            Err(error) => {
                if let Some(timing) = error.inference_timing() {
                    let response = arkavo_llm::ProviderResponse {
                        inference_timing: Some(timing.clone()),
                        ..Default::default()
                    };
                    let usage = self
                        .router
                        .attribute_response(model.clone(), estimated, &response);
                    budget
                        .record(&usage)
                        .await
                        .map_err(|e| Error::Other(e.into()))?;
                }
                Err(Error::Other(anyhow::anyhow!(
                    "Planning LLM call failed: {error}"
                )))
            }
        }
    }
}
