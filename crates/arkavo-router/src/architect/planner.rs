use super::{ArchitectPlan, ComplexityScore, Subtask, planning_provider, subtask_model};
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
        // What the planning call itself is expected to cost. Without a router
        // there is no pricing to ask for, and the plan carries no planning cost.
        let planning_cost = self
            .router
            .as_ref()
            .map_or(0.0, |router| router.usage_cost(&model, &preflight));
        if let Some(router) = self.router.as_ref() {
            // The planning arm is chosen from the configured providers, never
            // named by the caller, so the cloud gate gets no authorization.
            if let Some(budget) = budget {
                budget.check(planning_cost).await?;
            }
            router
                .authorize_call(&model, planning_cost, false, None)
                .await?;
        }

        let provider = self.planning_client(&model).await?;
        // complete_with_tools yields a ProviderResponse, which carries both
        // reasoning_content and the measured inference_timing the ledger needs.
        let result = provider
            .complete_with_tools(messages, None, Some(4096))
            .await;
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
        plan.planning_model = Some(model.clone());

        Self::estimate_costs(&mut plan, &model, planning_cost);

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
            let model = self.subtask_model(category);
            let cost = subtask_model::estimate_subtask_cost(&model, category);

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

    /// Arm for a subtask in `category`, filtered against what this device can
    /// run. See [`subtask_model::select_model_for_category`].
    fn subtask_model(&self, category: TaskCategory) -> ModelChoice {
        subtask_model::select_model_for_category(&self.availability, self.router.as_ref(), category)
    }

    /// Price the plan, and the single-arm baseline it is measured against.
    ///
    /// The baseline is every subtask run on the arm that actually planned —
    /// the only comparison the plan can support. Pricing it through
    /// [`subtask_model::estimate_subtask_cost`], the same function the
    /// subtasks are priced with, keeps the two sides of the subtraction on one
    /// scale.
    fn estimate_costs(plan: &mut ArchitectPlan, planning_model: &ModelChoice, planning_cost: f64) {
        let execution_cost: f64 = plan.subtasks.iter().map(|s| s.estimated_cost_usd).sum();
        plan.architect_estimate_usd = planning_cost + execution_cost;
        plan.single_arm_estimate_usd = plan
            .subtasks
            .iter()
            .map(|s| subtask_model::estimate_subtask_cost(planning_model, s.category))
            .sum();
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
        let model = planner.subtask_model(TaskCategory::FrontendUI);

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
        let model = planner.subtask_model(TaskCategory::BackendAPI);

        // Should prefer capable models for backend
        assert!(matches!(
            model,
            ModelChoice::ClaudeOpus
                | ModelChoice::GeminiPro
                | ModelChoice::LocalMinistral8B
                | ModelChoice::Gpt6Astra
        ));
    }

    #[cfg(feature = "openai")]
    #[test]
    fn mixed_cloud_providers_keep_subtasks_runnable() {
        for other in ["xai", "glm", "kimi", "deepseek"] {
            let mut availability = only(other);
            availability.openai = true;
            let planner = ArchitectPlanner::new().with_availability(availability);
            for category in [
                TaskCategory::FrontendUI,
                TaskCategory::BackendAPI,
                TaskCategory::Documentation,
                TaskCategory::CodeSearch,
            ] {
                let model = planner.subtask_model(category);
                assert!(!model.is_local(), "{other}: {category:?}");
                assert_ne!(
                    model,
                    ModelChoice::DeepSeekV32Speciale,
                    "subtasks need an execution model with tool support"
                );
                assert!(subtask_model::estimate_subtask_cost(&model, category) > 0.0);
            }
        }
    }

    #[tokio::test]
    async fn configured_cloud_preserves_cached_local_subtasks() {
        let availability = only("openai");
        let router = Router::new_offline()
            .await
            .unwrap()
            .with_selector(crate::ModelSelector::with_availability(
                availability.clone(),
                true,
            ))
            .await;
        let planner = ArchitectPlanner::new()
            .with_availability(availability)
            .with_router(Arc::new(router));
        assert_eq!(
            planner.subtask_model(TaskCategory::CodeSearch),
            ModelChoice::LocalQwen3
        );
    }

    /// An isolated planner fixture where OpenAI is the configured cloud provider —
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
        let router = router
            .with_cloud_policy(arkavo_budget::CloudPolicy::CloudWithinCap)
            .with_connectivity(crate::ConnectivityChecker::assume(true))
            .with_budget_tracker(tracker.clone())
            .with_provider_factory(provider.factory())
            .with_selector(crate::ModelSelector::with_availability(
                only("openai"),
                false,
            ))
            .await;
        Arc::new(router)
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
        assert_eq!(provider.output_limits(), vec![4096; 3]);

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

    /// Regression: the planner used to face the cloud policy before the
    /// ledger, so an exhausted cap under `LocalOnly` was reported as a policy
    /// denial. Planning asks the ledger first, like every other path.
    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn an_exhausted_cap_outranks_the_cloud_policy_on_planning() {
        let mut config = BudgetConfig::default();
        config.limits.session_limit = Some(TokenCost::from_cents(1));
        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());
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
            matches!(&error, Error::BudgetExceeded(_)),
            "the ledger must answer before the policy: got {error:?}"
        );
        assert_eq!(
            provider.builds(),
            0,
            "a refused plan must not open a client"
        );
        assert!(tracker.get_spending_history(10).await.is_empty());
    }

    /// The plan is measured against the arm that actually planned it, not a
    /// fixed Opus rate for a model the plan never touches.
    #[spec("ROUTER-010")]
    #[tokio::test]
    async fn the_savings_baseline_is_the_planning_arm() {
        let tracker = Arc::new(BudgetTracker::new(BudgetConfig::default()).await.unwrap());
        let provider = CountingProvider::new(TWO_STEP_PLAN);
        let router = astra_router(&tracker, &provider).await;

        let plan = astra_plan(&router).await.unwrap();
        let expected: f64 = plan
            .subtasks
            .iter()
            .map(|s| subtask_model::estimate_subtask_cost(&ModelChoice::Gpt6Astra, s.category))
            .sum();
        assert!(expected > 0.0);
        assert!(
            (plan.single_arm_estimate_usd - expected).abs() < 1e-12,
            "baseline {} should price the planning arm ({expected})",
            plan.single_arm_estimate_usd
        );
        assert!(
            plan.architect_estimate_usd > 0.0,
            "the planning call carries its own priced cost"
        );
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
