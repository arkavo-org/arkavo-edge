use crate::classifier::TaskClassifier;
use crate::decision::RoutingDecision;
use crate::metrics::RoutingMetrics;
use crate::selector::ModelSelector;
use crate::{Error, Result};
use arkavo_budget::TokenCost;
use arkavo_budget::tracker::{BudgetTracker, SpendingRecord};
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
        }
    }
}

pub struct CostOrchestrator {
    classifier: Arc<TaskClassifier>,
    selector: Arc<ModelSelector>,
    budget_tracker: Arc<BudgetTracker>,
    routing_metrics: Arc<RwLock<RoutingMetrics>>,
    orchestrator_metrics: Arc<RwLock<OrchestratorMetrics>>,
    budget_threshold: f64,
}

impl CostOrchestrator {
    pub async fn new(budget_tracker: Arc<BudgetTracker>) -> Result<Self> {
        let classifier = Arc::new(TaskClassifier::new().await?);
        let selector = Arc::new(ModelSelector::new());

        Ok(Self {
            classifier,
            selector,
            budget_tracker,
            routing_metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            orchestrator_metrics: Arc::new(RwLock::new(OrchestratorMetrics::new())),
            budget_threshold: 0.80,
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

        let estimated_token_cost = TokenCost::from_dollars(decision.estimated_cost_usd);

        let can_afford = self
            .budget_tracker
            .can_afford(agent_id, estimated_token_cost)
            .await
            .unwrap_or(true);

        if !can_afford {
            return Err(Error::BudgetExceeded(format!(
                "Agent {} cannot afford estimated cost ${:.4}",
                agent_id, decision.estimated_cost_usd
            )));
        }

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_budget::{BudgetConfig, BudgetManager};

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
