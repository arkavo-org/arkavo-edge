use crate::architect::{ArchitectExecutor, ArchitectPlanner, ArchitectResult, ComplexityScorer};
use crate::classifier::TaskClassifier;
use crate::decision::RoutingDecision;
use crate::metrics::RoutingMetrics;
use crate::selector::ModelSelector;
use crate::{Error, Result, Router};
use arkavo_budget::TokenCost;
use arkavo_budget::cost::TokenUsage;
use arkavo_budget::provider_costs::ProviderPricing;
use arkavo_budget::tracker::{ArchitectCostMetadata, BudgetTracker, SpendingRecord};
use arkavo_llm::Message;
use arkavo_mcp_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecommendation {
    pub suggestion: String,
    pub estimated_savings: f64,
    pub impact: String,
    pub priority: RecommendationPriority,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RecommendationPriority {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalingDecision {
    pub should_scale_down: bool,
    pub should_scale_up: bool,
    pub reasoning: String,
    pub current_usage_percent: f64,
    pub projected_usage_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorMetrics {
    pub total_orchestrated: u64,
    pub budget_switches: u64,
    pub recommendations_generated: u64,
    pub auto_scaling_decisions: u64,
    pub total_budget_saved: f64,
    /// Architect mode metrics
    pub architect_plans_executed: u64,
    pub architect_subtasks_completed: u64,
    pub architect_subtasks_failed: u64,
    pub architect_total_savings: f64,
    /// Provider error tracking
    pub provider_errors: std::collections::HashMap<String, u64>,
}

impl Default for OrchestratorMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl OrchestratorMetrics {
    pub fn new() -> Self {
        Self {
            total_orchestrated: 0,
            budget_switches: 0,
            recommendations_generated: 0,
            auto_scaling_decisions: 0,
            total_budget_saved: 0.0,
            architect_plans_executed: 0,
            architect_subtasks_completed: 0,
            architect_subtasks_failed: 0,
            architect_total_savings: 0.0,
            provider_errors: std::collections::HashMap::new(),
        }
    }

    /// Record a provider error for tracking
    pub fn record_provider_error(&mut self, provider: &str, error_type: &str) {
        let key = format!("{provider}:{error_type}");
        *self.provider_errors.entry(key).or_insert(0) += 1;
    }
}

/// Resolve the budget cost of a routing decision. Manifest-authored pricing is
/// authoritative when the model is in the table; otherwise the model's built-in
/// static estimate is used. This is the seam that makes editing a rate in the
/// manifest move the live budget gate (no code change, no runtime fetch).
fn estimate_decision_cost(pricing: &ProviderPricing, decision: &RoutingDecision) -> TokenCost {
    let est = decision.task_category.estimated_tokens();
    pricing
        .estimate_cost(
            decision.recommended_model.provider(),
            decision.recommended_model.name(),
            est.input,
            est.output,
        )
        .unwrap_or_else(|| TokenCost::from_dollars(decision.estimated_cost_usd))
}

pub struct CostOrchestrator {
    classifier: Arc<TaskClassifier>,
    selector: Arc<ModelSelector>,
    budget_tracker: Arc<BudgetTracker>,
    routing_metrics: Arc<RwLock<RoutingMetrics>>,
    orchestrator_metrics: Arc<RwLock<OrchestratorMetrics>>,
    budget_threshold: f64,
    /// Manifest-authored per-model pricing (cents per MTok). Empty by default;
    /// when a model is present it is the cost source of truth and overrides the
    /// model's built-in static estimate. Populated from the SwarmKit manifest
    /// at authoring time — never fetched from a vendor endpoint at runtime.
    pricing: ProviderPricing,
}

impl CostOrchestrator {
    pub async fn new(budget_tracker: Arc<BudgetTracker>) -> Result<Self> {
        Self::new_with_pricing(budget_tracker, ProviderPricing::new()).await
    }

    /// Construct with an authored pricing table (e.g. derived from a SwarmKit
    /// manifest's `pricing` block). Models present in the table are priced from
    /// it; everything else falls back to the built-in static estimate.
    pub async fn new_with_pricing(
        budget_tracker: Arc<BudgetTracker>,
        pricing: ProviderPricing,
    ) -> Result<Self> {
        let classifier = Arc::new(TaskClassifier::new().await?);
        let selector = Arc::new(ModelSelector::new());

        Ok(Self {
            classifier,
            selector,
            budget_tracker,
            routing_metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            orchestrator_metrics: Arc::new(RwLock::new(OrchestratorMetrics::new())),
            budget_threshold: 0.80,
            pricing,
        })
    }

    pub async fn route_with_budget(&self, task: &str, agent_id: &str) -> Result<RoutingDecision> {
        let classification = self.classifier.classify(task).await?;

        let budget_usage = self.calculate_budget_usage().await?;

        let decision = if budget_usage > self.budget_threshold {
            let switched_decision = self
                .selector
                .select_with_budget_constraint(&classification, task, budget_usage)
                .await?;

            let mut metrics = self.orchestrator_metrics.write().await;
            metrics.budget_switches += 1;
            drop(metrics);

            switched_decision
        } else {
            self.selector.select(&classification, task)?
        };

        let estimated_token_cost = estimate_decision_cost(&self.pricing, &decision);
        let estimated = decision.task_category.estimated_tokens();

        // Reserve the estimated cost atomically. `try_spend` holds the budget
        // lock across check-and-deduct, so two concurrent agents cannot both
        // pass against the same remaining budget and both dispatch — the race
        // a non-deducting `can_afford` check allowed by releasing its lock
        // before dispatch. Any reservation failure — an over-limit denial or a
        // budget-store error — fails closed: the route is denied, never
        // authorized. Engine-time actual usage reconciles this reservation
        // against the real spend (see `record_actual_spending`; issue #587).
        self.budget_tracker
            .try_spend(
                agent_id.to_string(),
                decision.recommended_model.provider().to_string(),
                decision.recommended_model.name().to_string(),
                TokenUsage::new(estimated.input, estimated.output),
                estimated_token_cost,
            )
            .await
            .map_err(|e| {
                // Fail closed on ANY reservation failure, but don't discard the
                // cause: an over-limit denial and a budget-store/persistence
                // error both deny here yet mean very different things to an
                // operator. Log and surface the underlying error instead of
                // collapsing every failure into a bare "cannot afford".
                tracing::warn!(error = %e, "budget reservation denied for agent {agent_id}");
                Error::BudgetExceeded(format!(
                    "Agent {agent_id} reservation denied for estimated cost ${:.4}: {e}",
                    decision.estimated_cost_usd
                ))
            })?;

        let mut routing_metrics = self.routing_metrics.write().await;
        routing_metrics.record_routing(&classification, &decision);
        drop(routing_metrics);

        let mut orch_metrics = self.orchestrator_metrics.write().await;
        orch_metrics.total_orchestrated += 1;
        drop(orch_metrics);

        Ok(decision)
    }

    pub async fn get_cost_recommendations(&self) -> Result<Vec<CostRecommendation>> {
        let mut recommendations = Vec::new();

        let budget_usage = self.calculate_budget_usage().await?;
        let routing_metrics = self.routing_metrics.read().await;

        if budget_usage > 0.70 {
            recommendations.push(CostRecommendation {
                suggestion: format!(
                    "Budget usage at {:.1}%. Consider using more local models.",
                    budget_usage * 100.0
                ),
                estimated_savings: routing_metrics.total_estimated_cost * 0.3,
                impact: "High cost reduction".to_string(),
                priority: RecommendationPriority::High,
            });
        }

        let local_usage = routing_metrics.local_model_usage_percent();
        if local_usage < 30.0 {
            recommendations.push(CostRecommendation {
                suggestion: format!(
                    "Only {local_usage:.1}% of tasks use local models. Increase local routing for code search and security tasks."
                ),
                estimated_savings: routing_metrics.total_estimated_cost * 0.4,
                impact: "Medium cost reduction, maintains quality".to_string(),
                priority: RecommendationPriority::Medium,
            });
        }

        if routing_metrics.average_cost() > 0.008 {
            recommendations.push(CostRecommendation {
                suggestion: format!(
                    "Average task cost ${:.4} is high. Consider context compression.",
                    routing_metrics.average_cost()
                ),
                estimated_savings: routing_metrics.total_estimated_cost * 0.2,
                impact: "Token reduction up to 60%".to_string(),
                priority: RecommendationPriority::Medium,
            });
        }

        let mut orch_metrics = self.orchestrator_metrics.write().await;
        orch_metrics.recommendations_generated += recommendations.len() as u64;

        Ok(recommendations)
    }

    pub async fn auto_scale_budget(&self, _agent_id: &str) -> Result<ScalingDecision> {
        let budget_usage = self.calculate_budget_usage().await?;
        let routing_metrics = self.routing_metrics.read().await;

        let average_cost = routing_metrics.average_cost();
        let projected_tasks = 100.0;
        let projected_cost = average_cost * projected_tasks;

        let status = self.budget_tracker.get_status().await;
        let session_limit = status
            .session_limit
            .map(|l| l.as_dollars())
            .unwrap_or(f64::MAX);
        let session_spent = status.session_spent.as_dollars();
        let projected_usage = (session_spent + projected_cost) / session_limit;

        let should_scale_down = budget_usage > 0.75 || projected_usage > 0.90;
        let should_scale_up = budget_usage < 0.30 && projected_usage < 0.50;

        let reasoning = if should_scale_down {
            format!(
                "Budget usage {:.1}%. Projected {:.1}% after 100 tasks. Recommend local models.",
                budget_usage * 100.0,
                projected_usage * 100.0
            )
        } else if should_scale_up {
            format!(
                "Budget usage {:.1}% with headroom. Can increase cloud model usage for better quality.",
                budget_usage * 100.0
            )
        } else {
            format!(
                "Budget usage {:.1}% is balanced. No scaling needed.",
                budget_usage * 100.0
            )
        };

        let mut orch_metrics = self.orchestrator_metrics.write().await;
        orch_metrics.auto_scaling_decisions += 1;

        Ok(ScalingDecision {
            should_scale_down,
            should_scale_up,
            reasoning,
            current_usage_percent: budget_usage * 100.0,
            projected_usage_percent: projected_usage * 100.0,
        })
    }

    pub async fn get_routing_metrics(&self) -> RoutingMetrics {
        self.routing_metrics.read().await.clone()
    }

    pub async fn get_orchestrator_metrics(&self) -> OrchestratorMetrics {
        self.orchestrator_metrics.read().await.clone()
    }

    async fn calculate_budget_usage(&self) -> Result<f64> {
        let status = self.budget_tracker.get_status().await;

        if let Some(session_limit) = status.session_limit {
            let limit_dollars = session_limit.as_dollars();
            if limit_dollars > 0.0 {
                return Ok((status.session_spent.as_dollars() / limit_dollars).clamp(0.0, 1.0));
            }
        }

        Ok(0.0)
    }

    /// Record engine-time actual spending for a routed call.
    ///
    /// `route_with_budget` now *reserves* the estimated cost up front (see the
    /// `try_spend` reservation there), so this must reconcile the reservation
    /// against the real usage — record the delta (`actual - estimated`), not a
    /// fresh add — or the call is billed twice. Wiring the live engine caller
    /// that performs that reconciliation is tracked by issue #587; until then
    /// this entry point has no production caller.
    ///
    /// Reconciliation is required because the reservation is **not** refunded
    /// on its own: a route whose downstream call fails, costs less than
    /// estimated, or never runs leaves the estimate reserved. The
    /// `route_with_budget` path (and `BudgetMiddleware`'s own actual-recording)
    /// are currently dormant — `CostOrchestrator` is constructed only in tests
    /// and `route_with_architect` has no production caller — so this cannot
    /// over-count or double-count today. Before wiring the path live (#587),
    /// the reconcile here must replace, not stack on, any middleware
    /// actual-recording for the same call.
    pub async fn record_actual_spending(
        &self,
        agent_id: String,
        provider: String,
        model: String,
        usage: arkavo_budget::cost::TokenUsage,
        cost: TokenCost,
    ) -> Result<SpendingRecord> {
        self.budget_tracker
            .record_spending(agent_id, provider, model, usage, cost)
            .await
            .map_err(|e| Error::BudgetError(e.to_string()))
    }

    pub fn with_budget_threshold(mut self, threshold: f64) -> Self {
        self.budget_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Check if a task should use architect mode based on complexity
    pub fn should_use_architect(&self, task: &str) -> bool {
        let scorer = ComplexityScorer::new();
        let complexity = scorer.analyze(task);
        complexity.architect_recommended
    }

    /// Route a task using architect mode for complex multi-step tasks
    /// This will auto-detect complexity and use architect mode when beneficial
    pub async fn route_with_architect(
        &self,
        task: &str,
        agent_id: &str,
        messages: Vec<Message>,
        tool_registry: Option<&ToolRegistry>,
    ) -> Result<ArchitectRoutingResult> {
        // Check complexity
        let scorer = ComplexityScorer::new();
        let complexity = scorer.analyze(task);

        if !complexity.architect_recommended {
            // Use standard routing for simple tasks
            let decision = self.route_with_budget(task, agent_id).await?;
            return Ok(ArchitectRoutingResult::Simple(decision));
        }

        // Check budget before starting architect mode
        let budget_usage = self.calculate_budget_usage().await?;
        if budget_usage > 0.95 {
            return Err(Error::BudgetExceeded(
                "Budget too low for architect mode. Use standard routing.".to_string(),
            ));
        }

        // Create architect plan
        let planner = ArchitectPlanner::new();
        let plan = planner
            .create_plan(task, complexity.clone())
            .await
            .map_err(|e| Error::ArchitectError(e.to_string()))?;

        // Record planning cost (estimate ~10% of total architect cost for planning)
        let planning_cost_usd = plan.architect_estimate_usd * 0.1;
        let planning_cost_metadata = ArchitectCostMetadata {
            plan_id: plan.id,
            phase: "planning".to_string(),
            subtask_index: None,
            subtask_id: None,
            opus_only_estimate: plan.opus_only_estimate_usd,
        };

        let planning_cost = TokenCost::from_dollars(planning_cost_usd);
        let input_tokens = complexity.estimated_output_tokens;
        let output_tokens = complexity.estimated_output_tokens / 2;
        let _ = self
            .budget_tracker
            .record_architect_spending(
                agent_id.to_string(),
                "anthropic".to_string(),
                "claude-opus".to_string(),
                arkavo_budget::cost::TokenUsage::new(input_tokens, output_tokens),
                planning_cost,
                planning_cost_metadata,
            )
            .await;

        // Execute the plan
        let router = self.create_router_for_executor().await?;
        let executor = ArchitectExecutor::new(Arc::new(router));
        let result = executor
            .execute(&plan, messages, tool_registry)
            .await
            .map_err(|e| Error::ArchitectError(e.to_string()))?;

        // Record execution costs for each subtask
        for subtask_result in &result.subtask_results {
            let opus_per_subtask = if !plan.subtasks.is_empty() {
                plan.opus_only_estimate_usd / plan.subtasks.len() as f64
            } else {
                0.0
            };

            let exec_metadata = ArchitectCostMetadata {
                plan_id: plan.id,
                phase: "execution".to_string(),
                subtask_index: Some(subtask_result.index),
                subtask_id: Some(subtask_result.subtask_id),
                opus_only_estimate: opus_per_subtask,
            };

            let subtask_cost = TokenCost::from_dollars(subtask_result.actual_cost_usd);
            let _ = self
                .budget_tracker
                .record_architect_spending(
                    agent_id.to_string(),
                    subtask_result.model_used.provider().to_string(),
                    subtask_result.model_used.name().to_string(),
                    arkavo_budget::cost::TokenUsage::new(0, 0),
                    subtask_cost,
                    exec_metadata,
                )
                .await;
        }

        // Update metrics
        let mut metrics = self.orchestrator_metrics.write().await;
        metrics.architect_plans_executed += 1;

        // Count successful vs failed subtasks
        let (completed, failed): (Vec<_>, Vec<_>) =
            result.subtask_results.iter().partition(|r| r.success);
        metrics.architect_subtasks_completed += completed.len() as u64;
        metrics.architect_subtasks_failed += failed.len() as u64;

        // Track provider errors from failed subtasks
        for failed_subtask in &failed {
            if let Some(error) = &failed_subtask.error {
                let error_type = categorize_provider_error(error);
                metrics.record_provider_error(failed_subtask.model_used.provider(), &error_type);
            }
        }

        metrics.architect_total_savings += result.actual_savings_usd;
        drop(metrics);

        Ok(ArchitectRoutingResult::Architect(result))
    }

    /// Create a minimal router for the executor
    async fn create_router_for_executor(&self) -> Result<Router> {
        Router::new().await
    }

    /// Get architect savings summary from budget tracker
    pub async fn get_architect_savings_summary(
        &self,
    ) -> arkavo_budget::tracker::ArchitectUsageSummary {
        self.budget_tracker.get_architect_summary().await
    }
}

/// Categorize provider errors for metrics tracking
fn categorize_provider_error(error: &str) -> String {
    let error_lower = error.to_lowercase();

    if error_lower.contains("rate limit") || error_lower.contains("429") {
        "rate_limit".to_string()
    } else if error_lower.contains("timeout") || error_lower.contains("timed out") {
        "timeout".to_string()
    } else if error_lower.contains("unauthorized")
        || error_lower.contains("401")
        || error_lower.contains("api key")
    {
        "auth_error".to_string()
    } else if error_lower.contains("500")
        || error_lower.contains("502")
        || error_lower.contains("503")
        || error_lower.contains("internal")
    {
        "server_error".to_string()
    } else if error_lower.contains("context")
        || error_lower.contains("token")
        || error_lower.contains("too long")
    {
        "context_limit".to_string()
    } else if error_lower.contains("connection")
        || error_lower.contains("network")
        || error_lower.contains("dns")
    {
        "network_error".to_string()
    } else if error_lower.contains("invalid") || error_lower.contains("malformed") {
        "invalid_request".to_string()
    } else {
        "unknown".to_string()
    }
}

/// Result of routing with architect support
#[derive(Debug)]
pub enum ArchitectRoutingResult {
    /// Simple routing decision for non-complex tasks
    Simple(RoutingDecision),
    /// Full architect result for complex multi-step tasks
    Architect(ArchitectResult),
}

impl ArchitectRoutingResult {
    /// Check if architect mode was used
    pub fn is_architect(&self) -> bool {
        matches!(self, Self::Architect(_))
    }

    /// Get estimated or actual total cost
    pub fn total_cost(&self) -> f64 {
        match self {
            Self::Simple(decision) => decision.estimated_cost_usd,
            Self::Architect(result) => result.actual_cost_usd,
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use arkavo_budget::{BudgetConfig, BudgetManager};

    #[test]
    fn manifest_pricing_is_authoritative_over_static_estimate() {
        use crate::classifier::TaskCategory;
        use crate::decision::ModelChoice;
        use arkavo_budget::provider_costs::PricingEntry;

        // Author GLM-5.2 at a deliberately inflated rate so the table-driven
        // cost is clearly distinct from the built-in static estimate.
        let mut pricing = ProviderPricing::new();
        pricing.register(&PricingEntry {
            model_id: "glm-5.2".to_string(),
            provider: "zhipu".to_string(),
            input_cents_per_mtok: 1400,
            output_cents_per_mtok: 4400,
            cached_input_cents_per_mtok: None,
            cache_write_cents_per_mtok: None,
            context_window: Some(1_000_000),
            max_output_tokens: Some(131_072),
        });
        let decision = RoutingDecision::new(
            ModelChoice::Glm52,
            TaskCategory::CodeGeneration,
            0.9,
            "task".to_string(),
        );
        // CodeGeneration estimates 800 in / 3000 out:
        // (800*1400 + 3000*4400)/1e6 = 1 + 13 = 14 cents.
        let cost = estimate_decision_cost(&pricing, &decision);
        assert_eq!(cost.as_cents(), 14, "authored table rate must drive cost");
        // And it must differ from the static estimate, proving the override.
        assert_ne!(cost, TokenCost::from_dollars(decision.estimated_cost_usd));
    }

    #[test]
    fn empty_pricing_falls_back_to_static_estimate() {
        use crate::classifier::TaskCategory;
        use crate::decision::ModelChoice;

        let pricing = ProviderPricing::new(); // no authored rates
        let decision = RoutingDecision::new(
            ModelChoice::Glm52,
            TaskCategory::CodeGeneration,
            0.9,
            "task".to_string(),
        );
        let cost = estimate_decision_cost(&pricing, &decision);
        assert_eq!(cost, TokenCost::from_dollars(decision.estimated_cost_usd));
    }

    #[tokio::test]
    async fn route_with_budget_reserves_the_estimated_cost() {
        // Regression for the cost-gate fail-open / TOCTOU race: the gate must
        // RESERVE the estimated cost atomically (try_spend, which holds the
        // budget lock across check-and-deduct), not run a non-deducting
        // can_afford check that releases its lock before dispatch — that race
        // lets two concurrent agents both pass against the same remaining
        // budget. Proof that a reservation actually happened: a successful
        // route leaves a spending record behind (true even for free local
        // models, where the reserved cost is $0 but the record is still
        // written). The old can_afford path recorded nothing.
        let config = BudgetConfig::default();
        let manager = BudgetManager::new(config).await.unwrap();
        let tracker = manager.tracker();

        let orchestrator = CostOrchestrator::new(tracker.clone()).await;
        if orchestrator.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        let orchestrator = orchestrator.unwrap();

        let decision = orchestrator
            .route_with_budget(
                "write a function that adds two numbers",
                "reservation-agent",
            )
            .await;
        assert!(
            decision.is_ok(),
            "route should succeed within the default budget: {decision:?}"
        );

        let history = tracker.get_spending_history(16).await;
        assert!(
            history.iter().any(|r| r.agent_id == "reservation-agent"),
            "a successful route must reserve the estimated cost (leaving a \
             spending record); found none — the gate checked without reserving"
        );
    }

    #[tokio::test]
    async fn test_cost_orchestrator_creation() {
        let config = BudgetConfig::default();
        let manager = BudgetManager::new(config).await.unwrap();
        let tracker = manager.tracker();

        let orchestrator = CostOrchestrator::new(tracker).await;
        if orchestrator.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        assert!(orchestrator.is_ok());
    }

    #[tokio::test]
    async fn test_budget_threshold_setting() {
        let config = BudgetConfig::default();
        let manager = BudgetManager::new(config).await.unwrap();
        let tracker = manager.tracker();

        let orchestrator = CostOrchestrator::new(tracker).await;
        if orchestrator.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        let orchestrator = orchestrator.unwrap().with_budget_threshold(0.90);

        assert_eq!(orchestrator.budget_threshold, 0.90);
    }

    #[tokio::test]
    async fn test_get_metrics() {
        let config = BudgetConfig::default();
        let manager = BudgetManager::new(config).await.unwrap();
        let tracker = manager.tracker();

        let orchestrator = CostOrchestrator::new(tracker).await;
        if orchestrator.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        let orchestrator = orchestrator.unwrap();

        let routing_metrics = orchestrator.get_routing_metrics().await;
        assert_eq!(routing_metrics.total_routes, 0);

        let orch_metrics = orchestrator.get_orchestrator_metrics().await;
        assert_eq!(orch_metrics.total_orchestrated, 0);
    }

    #[tokio::test]
    async fn test_auto_scale_budget() {
        let config = BudgetConfig::default();
        let manager = BudgetManager::new(config).await.unwrap();
        let tracker = manager.tracker();

        let orchestrator = CostOrchestrator::new(tracker).await;
        if orchestrator.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        let orchestrator = orchestrator.unwrap();

        let decision = orchestrator.auto_scale_budget("test-agent").await.unwrap();

        assert!(decision.current_usage_percent >= 0.0);
        assert!(!decision.reasoning.is_empty());
    }
}
