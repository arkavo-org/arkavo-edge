use super::{ArchitectPlan, ComplexityScore, Subtask, planning_provider};
use crate::classifier::TaskCategory;
use crate::decision::ModelChoice;
use crate::selector::ProviderAvailability;
use crate::{Error, Result, Router};
use arkavo_llm::{Message, Provider};
use serde::Deserialize;
use std::sync::Arc;

/// Creates execution plans by decomposing complex tasks using Opus
pub struct ArchitectPlanner {
    availability: ProviderAvailability,
    /// Accounting home for the planning call. Without it the plan is still
    /// produced, but the spend leaves no ledger entry (ASTRA-005), so every
    /// caller that has a router should attach it.
    router: Option<Arc<Router>>,
}

impl ArchitectPlanner {
    pub fn new() -> Self {
        Self {
            availability: ProviderAvailability::from_env(),
            router: None,
        }
    }

    /// Bill the planning call against the router's shared budget tracker, and
    /// resolve the planning client through that router.
    #[must_use]
    pub fn with_router(mut self, router: Arc<Router>) -> Self {
        self.router = Some(router);
        self
    }

    /// Plan against explicitly configured providers instead of reading the
    /// environment, so a caller that already knows its provider set (or is
    /// asserting behaviour) is not at the mercy of ambient API keys.
    #[must_use]
    pub fn with_availability(mut self, availability: ProviderAvailability) -> Self {
        self.availability = availability;
        self
    }

    /// The arm this planner would use, or `None` when nothing configured can
    /// plan. Architect mode is a cloud-planned optimisation, so a caller that
    /// has a cheaper path (`Router::route` falls back to standard routing)
    /// should ask this before committing: on a key-less local install there is
    /// no planner, and finding that out inside `create_plan` would cost a turn
    /// the cached local model can serve.
    pub fn planning_model(&self) -> Option<ModelChoice> {
        planning_provider::choose_model(&self.availability)
    }

    /// Create a plan using the best configured planning model.
    ///
    /// The gates run before the client exists: the shared ledger answers first
    /// (so an exhausted budget reports as `BudgetExceeded`, not as a policy
    /// denial), then the cloud policy. A refused plan therefore never opens a
    /// connection, and the caller sees the refusal rather than a downstream
    /// credential error. Usage is settled after the call against the model that
    /// actually served it.
    pub async fn create_plan(
        &self,
        task: &str,
        complexity: ComplexityScore,
    ) -> Result<ArchitectPlan> {
        let model = planning_provider::choose_model(&self.availability)
            .ok_or_else(planning_provider::no_planning_model)?;
        let messages = vec![Message::user(self.build_planning_prompt(task))];
        let preflight = crate::usage::estimate_request(&messages, None, 4096);
        let settled = crate::usage::estimate_request(&messages, None, 0);
        let budget = self.router.as_ref().and_then(|r| r.call_budget());
        if let Some(router) = self.router.as_ref() {
            // The planning arm is chosen from the configured providers, never
            // named by the caller, so the cloud gate gets no authorization.
            let estimated_cost = router.usage_cost(&model, &preflight);
            if let Some(budget) = budget {
                budget.check(estimated_cost).await?;
            }
            router.authorize_call(&model, estimated_cost, false).await?;
        }

        let provider = self.planning_client(&model).await?;
        // complete_with_tools yields a ProviderResponse, which carries both
        // reasoning_content and the measured inference_timing the ledger needs.
        let result = provider.complete_with_tools(messages, None, None).await;
        let response = match self.router.as_ref() {
            Some(router) => {
                router
                    .account_result(&model, &settled, result, budget)
                    .await
            }
            None => result.map_err(Error::Provider),
        }
        .map_err(|e| match e {
            passthrough @ (Error::BudgetExceeded(_)
            | Error::BudgetError(_)
            | Error::ModerationBlocked { .. }
            | Error::CloudConfirmationRequired { .. }) => passthrough,
            other => Error::ModelExecution(format!("Planning phase failed: {other}")),
        })?;

        let mut plan = self.parse_plan_response(task, &response.content, complexity)?;

        // Capture reasoning from thinking models (e.g., DeepSeek V3.2-Speciale)
        plan.planning_reasoning = response.reasoning_content;
        plan.planning_model = Some(model);

        // Calculate cost estimates
        self.estimate_costs(&mut plan);

        Ok(plan)
    }

    /// Client for the planning arm. With a router attached it comes from the
    /// router's own construction path, so planning cannot diverge from the rest
    /// of routing (and inherits any provider substitution installed there).
    async fn planning_client(&self, model: &ModelChoice) -> Result<Box<dyn Provider>> {
        match self.router.as_ref() {
            Some(router) => Ok(router.get_provider_attributed(model).await?.0),
            None => planning_provider::build(model),
        }
    }

    fn build_planning_prompt(&self, task: &str) -> String {
        format!(
            r#"You are an expert software architect. Analyze this task and break it into concrete subtasks.

Task: {task}

For each subtask, specify:
1. A clear description of what needs to be done
2. The category (one of: frontend_ui, backend_api, test_generation, documentation, security_scan, refactoring, code_generation)
3. Dependencies (indices of subtasks that must complete first, 0-indexed)

Respond with ONLY valid JSON in this exact format:
{{
  "subtasks": [
    {{
      "description": "Brief description of the subtask",
      "category": "category_name",
      "dependencies": []
    }}
  ]
}}

Guidelines:
- Keep subtasks focused and atomic
- Order subtasks logically (dependencies first)
- Use 3-8 subtasks for most tasks
- Backend tasks should precede frontend tasks that depend on them
- Tests should come after the code they test"#
        )
    }

    fn parse_plan_response(
        &self,
        original_task: &str,
        response: &str,
        complexity: ComplexityScore,
    ) -> Result<ArchitectPlan> {
        // Try to extract JSON from the response
        let json_str = self.extract_json(response)?;

        #[derive(Deserialize)]
        struct PlanResponse {
            subtasks: Vec<SubtaskResponse>,
        }

        #[derive(Deserialize)]
        struct SubtaskResponse {
            description: String,
            category: String,
            #[serde(default)]
            dependencies: Vec<usize>,
        }

        let parsed: PlanResponse = serde_json::from_str(&json_str)
            .map_err(|e| Error::Classification(format!("Failed to parse plan JSON: {e}")))?;

        if parsed.subtasks.is_empty() {
            return Err(Error::Classification(
                "Plan contains no subtasks".to_string(),
            ));
        }

        let mut plan = ArchitectPlan::new(original_task.to_string(), complexity);

        for (index, subtask_resp) in parsed.subtasks.iter().enumerate() {
            let category = TaskCategory::from_string(&subtask_resp.category);
            let model = self.select_model_for_category(category);
            let cost = self.estimate_subtask_cost(&model, category);

            let subtask = Subtask::new(index, subtask_resp.description.clone(), category)
                .with_model(model, cost)
                .with_dependencies(subtask_resp.dependencies.clone());

            plan.add_subtask(subtask);
        }

        Ok(plan)
    }

    fn extract_json(&self, response: &str) -> Result<String> {
        // Try to find JSON object in the response
        if let Some(start) = response.find('{')
            && let Some(end) = response.rfind('}')
        {
            return Ok(response[start..=end].to_string());
        }

        // If no JSON found, return error
        Err(Error::Classification(
            "No valid JSON found in planning response".to_string(),
        ))
    }

    /// Select the best model for a subtask category
    fn select_model_for_category(&self, category: TaskCategory) -> ModelChoice {
        if self.availability.openai
            && !self.availability.anthropic
            && !self.availability.gemini
            && !self.availability.deepseek
            && !self.availability.kimi
            && !self.availability.glm
            && !self.availability.xai
        {
            return ModelChoice::Gpt6Astra;
        }
        match category {
            // Frontend tasks: Use cheaper, fast models
            TaskCategory::FrontendUI => {
                if self.availability.gemini {
                    ModelChoice::Gemini35Flash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }

            // Backend/Security/Tests/Review: Use more capable models
            TaskCategory::BackendAPI
            | TaskCategory::SecurityScan
            | TaskCategory::TestGeneration
            | TaskCategory::CodeReview => {
                if self.availability.anthropic {
                    ModelChoice::ClaudeOpus
                } else if self.availability.gemini {
                    ModelChoice::GeminiPro
                } else {
                    ModelChoice::LocalMinistral8B
                }
            }

            // Documentation: Use cheaper models
            TaskCategory::Documentation => {
                if self.availability.gemini {
                    ModelChoice::Gemini35Flash
                } else {
                    ModelChoice::LocalQwen3
                }
            }

            // Refactoring: Use balanced models
            TaskCategory::Refactoring | TaskCategory::CodeGeneration => {
                if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else if self.availability.gemini {
                    ModelChoice::GeminiPro
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }

            // Code search: Local model is sufficient
            TaskCategory::CodeSearch => ModelChoice::LocalQwen3,

            // Vision: Needs multimodal
            TaskCategory::VisionAnalysis => {
                if self.availability.gemini {
                    ModelChoice::Gemini35Flash
                } else if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else {
                    ModelChoice::LocalMinistral3B
                }
            }

            // Game/simulation and general: Use balanced default
            TaskCategory::GameSimulation | TaskCategory::General => {
                if self.availability.anthropic {
                    ModelChoice::ClaudeSonnet
                } else if self.availability.gemini {
                    ModelChoice::Gemini35Flash
                } else {
                    ModelChoice::LocalQwen3
                }
            }
        }
    }

    fn estimate_subtask_cost(&self, model: &ModelChoice, category: TaskCategory) -> f64 {
        let token_estimate = category.estimated_tokens();

        match model {
            ModelChoice::Gpt6Astra => crate::RoutingDecision::estimate_cost(model, category),
            ModelChoice::GeminiFlash => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 0.30;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 2.50;
                input_cost + output_cost
            }
            ModelChoice::Gemini35Flash => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 1.50;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 9.00;
                input_cost + output_cost
            }
            ModelChoice::GeminiPro => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 1.25;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 5.00;
                input_cost + output_cost
            }
            ModelChoice::ClaudeSonnet => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 3.00;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 15.00;
                input_cost + output_cost
            }
            ModelChoice::ClaudeOpus => {
                // Opus 4.8 pricing ($5/$25); the old $15/$75 was Opus 4.1.
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 5.00;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 25.00;
                input_cost + output_cost
            }
            ModelChoice::ClaudeFable5 => {
                let input_cost = (token_estimate.input as f64 / 1_000_000.0) * 10.00;
                let output_cost = (token_estimate.output as f64 / 1_000_000.0) * 50.00;
                input_cost + output_cost
            }
            _ => 0.0, // Local models are free
        }
    }

    fn estimate_costs(&self, plan: &mut ArchitectPlan) {
        // Calculate architect mode total cost
        let planning_cost = 0.005; // ~200 output tokens from Opus at $25/1M
        let execution_cost: f64 = plan.subtasks.iter().map(|s| s.estimated_cost_usd).sum();
        plan.architect_estimate_usd = planning_cost + execution_cost;

        // Calculate Opus-only estimate (Opus 4.8: $5/$25 per MTok)
        let total_output_tokens: u32 = plan
            .subtasks
            .iter()
            .map(|s| s.category.estimated_tokens().output)
            .sum();
        let total_input_tokens = total_output_tokens / 3;
        let input_cost = (total_input_tokens as f64 / 1_000_000.0) * 5.00;
        let output_cost = (total_output_tokens as f64 / 1_000_000.0) * 25.00;
        plan.opus_only_estimate_usd = input_cost + output_cost;
    }
}

impl Default for ArchitectPlanner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architect::executor::ArchitectExecutor;
    use crate::test_support::{CountingProvider, only};
    use arkavo_budget::{BudgetConfig, BudgetTracker, TokenCost};
    use arkavo_test_macros::spec;

    const TWO_STEP_PLAN: &str = r#"{"subtasks":[
        {"description":"first step","category":"general","dependencies":[]},
        {"description":"second step","category":"general","dependencies":[]}]}"#;

    #[spec("ROUTER-010")]
    #[test]
    fn test_extract_json() {
        let planner = ArchitectPlanner::new();

        let response = r#"Here is the plan:
        {"subtasks": [{"description": "test", "category": "frontend_ui", "dependencies": []}]}
        That's the plan."#;

        let json = planner.extract_json(response).unwrap();
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
    }

    #[spec("ROUTER-010")]
    #[test]
    fn test_model_selection_frontend() {
        let planner = ArchitectPlanner::new();
        let model = planner.select_model_for_category(TaskCategory::FrontendUI);

        // Should prefer cheaper models for frontend
        assert!(matches!(
            model,
            ModelChoice::GeminiFlash
                | ModelChoice::Gemini35Flash
                | ModelChoice::ClaudeSonnet
                | ModelChoice::LocalMinistral3B
                | ModelChoice::Gpt6Astra
        ));
    }

    #[spec("ROUTER-010")]
    #[test]
    fn test_model_selection_backend() {
        let planner = ArchitectPlanner::new();
        let model = planner.select_model_for_category(TaskCategory::BackendAPI);

        // Should prefer capable models for backend
        assert!(matches!(
            model,
            ModelChoice::ClaudeOpus
                | ModelChoice::GeminiPro
                | ModelChoice::LocalMinistral8B
                | ModelChoice::Gpt6Astra
        ));
    }

    /// A cloud-only install where OpenAI is the single configured provider —
    /// the deployment shape that made every subtask pick Astra.
    fn astra_planner(router: Arc<Router>) -> ArchitectPlanner {
        ArchitectPlanner {
            availability: only("openai"),
            router: Some(router),
        }
    }

    async fn astra_router(
        tracker: &Arc<BudgetTracker>,
        provider: &CountingProvider,
    ) -> Arc<Router> {
        let mut router = Router::new_offline().await.unwrap();
        router.set_offline_mode(false);
        Arc::new(
            router
                .with_cloud_policy(arkavo_budget::CloudPolicy::CloudWithinCap)
                .with_connectivity(crate::ConnectivityChecker::assume(true))
                .with_budget_tracker(tracker.clone())
                .with_provider_factory(provider.factory()),
        )
    }

    async fn astra_plan(router: &Arc<Router>) -> Result<ArchitectPlan> {
        astra_planner(router.clone())
            .create_plan("ship the feature", ComplexityScore::simple())
            .await
    }

    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn planning_and_every_subtask_reach_the_shared_ledger() {
        let tracker = Arc::new(BudgetTracker::new(BudgetConfig::default()).await.unwrap());
        let provider = CountingProvider::new(TWO_STEP_PLAN);
        let router = astra_router(&tracker, &provider).await;

        let plan = astra_plan(&router).await.unwrap();
        assert_eq!(plan.subtasks.len(), 2);
        assert_eq!(plan.planning_model, Some(ModelChoice::Gpt6Astra));
        assert!(
            plan.subtasks
                .iter()
                .all(|s| s.assigned_model == ModelChoice::Gpt6Astra)
        );

        let result = ArchitectExecutor::new(router)
            .execute(&plan, Vec::new(), None)
            .await
            .unwrap();
        assert!(result.subtask_results.iter().all(|r| r.success));
        assert_eq!(
            provider.calls(),
            3,
            "one planning call plus one per subtask"
        );
        assert!(result.actual_cost_usd > 0.0);

        let history = tracker.get_spending_history(10).await;
        assert_eq!(history.len(), 3, "planning plus one entry per subtask");
        assert!(
            history
                .iter()
                .all(|e| e.model == "gpt-6-astra" && e.provider == "openai")
        );
    }

    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn exhausted_budget_stops_the_next_subtask_before_it_spends() {
        let mut config = BudgetConfig::default();
        // Funds the planning call and the first subtask, not the second.
        config.limits.session_limit = Some(TokenCost::from_cents(100));
        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());
        let provider = CountingProvider::new(TWO_STEP_PLAN);
        let router = astra_router(&tracker, &provider).await;

        let plan = astra_plan(&router).await.unwrap();
        let error = ArchitectExecutor::new(router)
            .execute(&plan, Vec::new(), None)
            .await
            .unwrap_err();
        assert!(matches!(error, Error::BudgetExceeded(_)), "got {error:?}");
        assert_eq!(
            provider.calls(),
            2,
            "the refused subtask is never dispatched"
        );
        assert_eq!(tracker.get_spending_history(10).await.len(), 2);
    }

    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn a_failed_attempt_at_the_ceiling_is_not_redispatched() {
        let tracker = Arc::new(BudgetTracker::new(BudgetConfig::default()).await.unwrap());
        // First subtask succeeds, second fails with no rung above it.
        let provider = CountingProvider::failing_from("done", 1);
        let router = astra_router(&tracker, &provider).await;
        let mut plan = ArchitectPlan::new("ship the feature".into(), ComplexityScore::simple());
        for index in 0..2 {
            plan.add_subtask(
                Subtask::new(index, format!("step {index}"), TaskCategory::General)
                    .with_model(ModelChoice::ClaudeFable5, 0.0),
            );
        }

        let result = ArchitectExecutor::new(router)
            .execute(&plan, Vec::new(), None)
            .await
            .unwrap();
        assert_eq!(provider.calls(), 2, "no retry against the same model");
        assert!(!result.subtask_results[1].success);
        assert_eq!(result.subtask_results[1].retry_count, 1);
        assert!(
            result.subtask_results[1]
                .error
                .as_deref()
                .unwrap()
                .contains("no available escalation target")
        );
        // The failed attempt reported usage, so it stays charged.
        assert_eq!(tracker.get_spending_history(10).await.len(), 2);
    }

    /// Planning is a paid cloud call too, so the gates must run before the
    /// planning client is built — otherwise a refused plan surfaces as a
    /// credential error from a connection that should never have been opened.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn local_only_denies_the_planning_call() {
        let tracker = Arc::new(BudgetTracker::new(BudgetConfig::default()).await.unwrap());
        let provider = CountingProvider::new(TWO_STEP_PLAN);
        let mut router = Router::new_offline().await.unwrap();
        router.set_offline_mode(false);
        let router = Arc::new(
            router
                .with_cloud_policy(arkavo_budget::CloudPolicy::LocalOnly)
                .with_connectivity(crate::ConnectivityChecker::assume(true))
                .with_budget_tracker(tracker.clone())
                .with_provider_factory(provider.factory()),
        );

        let error = astra_plan(&router).await.unwrap_err();
        assert!(
            matches!(&error, Error::ModerationBlocked { policy_id, .. } if policy_id == "cloud_spend"),
            "got {error:?}"
        );
        assert_eq!(provider.builds(), 0, "a denied plan must not open a client");
        assert_eq!(provider.calls(), 0);
        assert!(tracker.get_spending_history(10).await.is_empty());
    }

    /// Architect subtasks spend like any other cloud call, so the executor's
    /// own provider resolution has to face the cloud policy too.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn local_only_denies_a_cloud_subtask() {
        let tracker = Arc::new(BudgetTracker::new(BudgetConfig::default()).await.unwrap());
        let provider = CountingProvider::new("done");
        let mut router = Router::new_offline().await.unwrap();
        router.set_offline_mode(false);
        let router = Arc::new(
            router
                .with_cloud_policy(arkavo_budget::CloudPolicy::LocalOnly)
                .with_connectivity(crate::ConnectivityChecker::assume(true))
                .with_budget_tracker(tracker.clone())
                .with_provider_factory(provider.factory()),
        );
        let mut plan = ArchitectPlan::new("ship the feature".into(), ComplexityScore::simple());
        plan.add_subtask(
            Subtask::new(0, "only step".into(), TaskCategory::General)
                .with_model(ModelChoice::Gpt6Astra, 0.0),
        );

        let error = ArchitectExecutor::new(router)
            .execute(&plan, Vec::new(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, Error::ModerationBlocked { policy_id, .. } if policy_id == "cloud_spend"),
            "got {error:?}"
        );
        assert_eq!(provider.builds(), 0);
        assert_eq!(provider.calls(), 0);
        assert!(tracker.get_spending_history(10).await.is_empty());
    }
}
