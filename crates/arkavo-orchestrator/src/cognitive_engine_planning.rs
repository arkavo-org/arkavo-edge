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

        let messages = vec![LlmMessage::user(planning_prompt.clone())];
        let max_tokens = planner_config.max_tokens().unwrap_or(4096);
        let budget = CallBudget {
            tracker: &self.budget_tracker,
            agent_id: "github-orchestrator",
        };

        // Both gates settle before the planning client is built, so a refusal
        // never opens a connection and the caller sees the policy error rather
        // than a downstream credential failure. The ledger answers first, so an
        // exhausted cap reports as `BudgetExceeded`. The preflight prices the
        // schema in unconditionally — the upper bound of what this call can
        // cost, since whether the provider takes a schema is not knowable until
        // it exists. The planning arm is routed, never named by a caller, so the
        // cloud gate is asked without authorization.
        let schema = JsonExecutionPlan::json_schema();
        let preflight = estimate_request(&messages, Some(&schema), max_tokens as u32);
        let preflight_cost = self
            .router
            .usage_cost(&decision.recommended_model, &preflight);
        // The arm is routed, never named, so an unprovisioned local weight is a
        // refusal rather than a multi-gigabyte download inside the plan.
        self.router
            .require_provisioned(&decision.recommended_model)
            .map_err(|e| Error::Other(e.into()))?;
        budget
            .check(preflight_cost)
            .await
            .map_err(|e| Error::Other(e.into()))?;
        self.router
            .authorize_call(&decision.recommended_model, preflight_cost, false, None)
            .await
            .map_err(|e| Error::Other(e.into()))?;

        let (planning_provider, actual_model) = self
            .router
            .get_provider_attributed(&decision.recommended_model)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Planning provider unavailable: {e}")))?;

        // Use structured output with JSON schema if provider supports it
        let schema = planning_provider
            .supports_structured_output()
            .then_some(schema);
        let estimated = estimate_request(&messages, schema.as_ref(), max_tokens as u32);
        let response = planning_provider
            .complete_with_schema_response(messages, schema, Some(max_tokens))
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

        let messages = vec![LlmMessage::user(adjustment_prompt.clone())];

        let estimated = estimate_request(&messages, None, 4096);
        let budget = CallBudget {
            tracker: &self.budget_tracker,
            agent_id: "github-orchestrator",
        };
        // Ledger then policy, both before the client exists — as on every other
        // routing path, so a refused adjustment opens no connection.
        let estimated_cost = self
            .router
            .usage_cost(&decision.recommended_model, &estimated);
        self.router
            .require_provisioned(&decision.recommended_model)
            .map_err(|e| Error::Other(e.into()))?;
        budget
            .check(estimated_cost)
            .await
            .map_err(|e| Error::Other(e.into()))?;
        self.router
            .authorize_call(&decision.recommended_model, estimated_cost, false, None)
            .await
            .map_err(|e| Error::Other(e.into()))?;

        let (provider, actual_model) = self
            .router
            .get_provider_attributed(&decision.recommended_model)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Adjustment provider unavailable: {e}")))?;

        let response = provider
            .complete_with_schema_response(messages, None, Some(4096))
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

#[cfg(all(test, feature = "openai"))]
mod tests {
    use super::*;
    use arkavo_budget::{BudgetConfig, CloudPolicy};
    use arkavo_llm::{Message, Provider};
    use arkavo_router::{ModelChoice, ProviderFactory};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts both halves of a dispatch: providers the router asked this
    /// factory to build, and calls that actually reached a model. A refusal
    /// must leave both at zero — counting only calls would pass even when the
    /// gate ran after a client was already open.
    #[derive(Clone, Default)]
    struct DispatchCounter {
        builds: Arc<AtomicUsize>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Provider for DispatchCounter {
        async fn complete_with_options(
            &self,
            _: Vec<Message>,
            _: Option<usize>,
        ) -> arkavo_llm::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(arkavo_llm::Error::Provider("unexpected dispatch".into()))
        }

        async fn stream(
            &self,
            _: Vec<Message>,
        ) -> arkavo_llm::Result<
            Box<
                dyn tokio_stream::Stream<Item = arkavo_llm::Result<arkavo_llm::StreamResponse>>
                    + Send
                    + Unpin,
            >,
        > {
            panic!("planning does not stream")
        }

        fn name(&self) -> &str {
            "dispatch-counter"
        }
    }

    impl ProviderFactory for DispatchCounter {
        fn build(&self, _: &ModelChoice) -> arkavo_router::Result<Box<dyn Provider>> {
            self.builds.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(self.clone()))
        }
    }

    fn assignment() -> AgentAssignment {
        serde_json::from_value(serde_json::json!({
            "issue_number": 1, "repository": "test/repo", "issue_title": "Fix a bug",
            "issue_body": "Private issue content", "assigned_agent_id": null,
            "assignment_rationale": "test",
            "routing_decision": {
                "strategy": "plan_first", "rationale": "test", "should_notify_human": false,
                "priority": "medium", "analysis": {
                    "issue_type": "bug", "complexity": "simple", "technologies": [],
                    "required_capabilities": [], "estimated_tokens": 1000
                }
            }
        }))
        .unwrap()
    }

    fn step() -> PlanStep {
        PlanStep {
            step_number: 1,
            description: "Fix a bug".into(),
            commands: vec![],
            verification: vec![],
            confidence: 0.5,
        }
    }

    fn failures() -> [VerificationResult; 1] {
        [VerificationResult {
            check: crate::cognitive_engine_core::VerificationCheck::TestsPassing,
            passed: false,
            details: "Private failure details".into(),
        }]
    }

    /// A planner with one configured cloud provider, no local weights on disk,
    /// and every provider substituted — so nothing here reads credentials, the
    /// model cache or the network.
    async fn planner_with(
        policy: CloudPolicy,
        config: BudgetConfig,
    ) -> (Planner, DispatchCounter, Arc<BudgetTracker>) {
        planner_for(
            policy,
            config,
            arkavo_router::ProviderAvailability {
                openai: true,
                ..Default::default()
            },
        )
        .await
    }

    async fn planner_for(
        policy: CloudPolicy,
        config: BudgetConfig,
        availability: arkavo_router::ProviderAvailability,
    ) -> (Planner, DispatchCounter, Arc<BudgetTracker>) {
        let counter = DispatchCounter::default();
        let tracker = Arc::new(BudgetTracker::new(config).await.unwrap());
        let mut router = Router::new_offline().await.unwrap();
        router.set_offline_mode(false);
        let router = router
            .with_selector(arkavo_router::ModelSelector::with_availability(
                availability,
                false,
            ))
            .await
            .with_connectivity(arkavo_router::ConnectivityChecker::assume(true))
            .with_cloud_policy(policy)
            .with_provider_factory(Arc::new(counter.clone()));
        let planner = Planner::new(tracker.clone(), Arc::new(router), None);
        (planner, counter, tracker)
    }

    fn router_error(error: Error) -> anyhow::Error {
        let Error::Other(error) = error else {
            panic!("unexpected error")
        };
        error
    }

    #[tokio::test]
    async fn cloud_policy_blocks_planning_and_adjustment_before_dispatch() {
        for policy in [CloudPolicy::LocalOnly, CloudPolicy::AskBeforeCloud] {
            let (planner, counter, tracker) = planner_with(policy, BudgetConfig::default()).await;
            for error in [
                planner.plan(&assignment()).await.unwrap_err(),
                planner.adjust(&step(), &failures()).await.unwrap_err(),
            ] {
                let error = router_error(error);
                let error = error
                    .downcast_ref::<arkavo_router::Error>()
                    .expect("router error");
                assert!(
                    matches!(
                        error,
                        arkavo_router::Error::ModerationBlocked { .. }
                            | arkavo_router::Error::CloudConfirmationRequired { .. }
                    ),
                    "{error}"
                );
            }
            assert_eq!(
                counter.builds.load(Ordering::SeqCst),
                0,
                "a refused plan must not open a client"
            );
            assert_eq!(counter.calls.load(Ordering::SeqCst), 0);
            assert!(tracker.get_spending_history(10).await.is_empty());
        }
    }

    /// Regression: the ledger answers before the cloud policy, so an exhausted
    /// cap reports the money being gone rather than a policy denial — which
    /// would have sent the operator looking for the wrong setting.
    #[tokio::test]
    async fn an_exhausted_cap_outranks_the_cloud_policy() {
        let mut config = BudgetConfig::default();
        config.limits.session_limit = Some(arkavo_budget::TokenCost::from_cents(1));
        let (planner, counter, tracker) = planner_with(CloudPolicy::LocalOnly, config).await;

        for error in [
            planner.plan(&assignment()).await.unwrap_err(),
            planner.adjust(&step(), &failures()).await.unwrap_err(),
        ] {
            let error = router_error(error);
            let error = error
                .downcast_ref::<arkavo_router::Error>()
                .expect("router error");
            assert!(
                matches!(error, arkavo_router::Error::BudgetExceeded(_)),
                "the ledger must answer before the policy: {error}"
            );
        }
        assert_eq!(
            counter.builds.load(Ordering::SeqCst),
            0,
            "a refused plan must not open a client"
        );
        assert_eq!(counter.calls.load(Ordering::SeqCst), 0);
        assert!(tracker.get_spending_history(10).await.is_empty());
    }

    /// Regression: `plan` and `adjust` took their arm from classification and
    /// went straight to provider construction. With no cloud keys the routed
    /// arm is local, and on a device holding no weights that reached the
    /// loader and started a multi-gigabyte download mid-plan. The
    /// cloud-policy test above cannot see this: it configures OpenAI, so the
    /// routed arm is never local.
    #[tokio::test]
    async fn an_unprovisioned_device_refuses_planning_and_adjustment() {
        let (planner, counter, tracker) = planner_for(
            CloudPolicy::CloudWithinCap,
            BudgetConfig::default(),
            arkavo_router::ProviderAvailability::default(),
        )
        .await;

        for error in [
            planner.plan(&assignment()).await.unwrap_err(),
            planner.adjust(&step(), &failures()).await.unwrap_err(),
        ] {
            let error = router_error(error);
            let error = error
                .downcast_ref::<arkavo_router::Error>()
                .expect("router error");
            assert!(
                matches!(error, arkavo_router::Error::ModelNotAvailable { .. }),
                "{error}"
            );
        }
        assert_eq!(
            counter.builds.load(Ordering::SeqCst),
            0,
            "a refused plan must not open a client"
        );
        assert_eq!(counter.calls.load(Ordering::SeqCst), 0);
        assert!(tracker.get_spending_history(10).await.is_empty());
    }
}
