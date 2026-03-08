use crate::roi_metrics::{CostMetrics, HourlyCost, ROICalculator, ROIDashboard};
use crate::types::AgUiEvent;
use arkavo_router::{CostOrchestrator, WorkflowCostPredictor};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

pub struct CostHandler {
    orchestrator: Option<Arc<RwLock<CostOrchestrator>>>,
    event_tx: Option<mpsc::Sender<AgUiEvent>>,
    budget_manager: Option<Arc<arkavo_budget::BudgetManager>>,
}

impl Default for CostHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CostHandler {
    pub fn new() -> Self {
        Self {
            orchestrator: None,
            event_tx: None,
            budget_manager: None,
        }
    }

    pub fn set_budget_manager(&mut self, manager: Arc<arkavo_budget::BudgetManager>) {
        self.budget_manager = Some(manager);
    }

    pub fn with_orchestrator(
        mut self,
        orchestrator: Arc<RwLock<CostOrchestrator>>,
        event_tx: mpsc::Sender<AgUiEvent>,
    ) -> Self {
        self.orchestrator = Some(orchestrator);
        self.event_tx = Some(event_tx);
        self
    }

    pub async fn handle_event(
        &self,
        event: &AgUiEvent,
        tx: &mpsc::Sender<AgUiEvent>,
    ) -> anyhow::Result<()> {
        match event {
            AgUiEvent::GetCostMetrics { time_range } => {
                let metrics = self.get_cost_metrics(time_range).await?;
                let response = AgUiEvent::CostMetricsUpdate {
                    metrics,
                    event_id: uuid::Uuid::new_v4().to_string(),
                };
                tx.send(response).await?;
            }

            AgUiEvent::GetROIDashboard => {
                let dashboard = self.get_roi_dashboard().await?;
                let response = AgUiEvent::ROIDashboardUpdate {
                    dashboard,
                    event_id: uuid::Uuid::new_v4().to_string(),
                };
                tx.send(response).await?;
            }

            AgUiEvent::GetCostPrediction { tasks } => {
                let prediction = self.predict_workflow_cost(tasks).await?;
                let response = AgUiEvent::CostPredictionUpdate {
                    prediction,
                    event_id: uuid::Uuid::new_v4().to_string(),
                };
                tx.send(response).await?;
            }

            _ => {}
        }

        Ok(())
    }

    async fn get_cost_metrics(&self, time_range: &str) -> anyhow::Result<CostMetrics> {
        let Some(orchestrator) = self.orchestrator.as_ref() else {
            return Ok(CostMetrics {
                time_range: time_range.to_string(),
                total_cost: 0.0,
                total_savings: 0.0,
                cost_by_hour: vec![],
                average_task_cost: 0.0,
            });
        };

        let orch_guard = orchestrator.read().await;
        let routing_metrics = orch_guard.get_routing_metrics().await;

        let total_cost = routing_metrics.total_estimated_cost;
        let total_savings = routing_metrics.total_cost_saved;
        let average_task_cost = routing_metrics.average_cost();

        let cost_by_hour = vec![HourlyCost {
            hour: chrono::Utc::now().format("%H:00").to_string(),
            cost: total_cost,
            tasks: routing_metrics.total_routes,
        }];

        Ok(CostMetrics {
            time_range: time_range.to_string(),
            total_cost,
            total_savings,
            cost_by_hour,
            average_task_cost,
        })
    }

    async fn get_roi_dashboard(&self) -> anyhow::Result<ROIDashboard> {
        let (routing_metrics, orch_metrics) = if let Some(orchestrator) = self.orchestrator.as_ref()
        {
            let orch_guard = orchestrator.read().await;
            (
                orch_guard.get_routing_metrics().await,
                orch_guard.get_orchestrator_metrics().await,
            )
        } else {
            (
                arkavo_router::RoutingMetrics::default(),
                arkavo_router::OrchestratorMetrics::default(),
            )
        };

        let (session_limit, session_spent, agent_allocations) =
            if let Some(ref manager) = self.budget_manager {
                let config = manager.get_config().await;
                let tracker = manager.tracker();
                let status = tracker.get_status().await;

                let mut allocations = Vec::new();
                for (agent_id, agent_budget) in &config.agent_budgets {
                    let agent_status = tracker.get_agent_status(agent_id).await;
                    let spent = agent_status
                        .as_ref()
                        .map(|s| s.session_spent.as_dollars())
                        .unwrap_or(0.0);
                    let allocated = agent_budget
                        .session_limit
                        .map(|l| l.as_dollars())
                        .unwrap_or(0.0);

                    allocations.push(arkavo_budget::AgentAllocationSummary {
                        agent_id: agent_id.clone(),
                        allocated_usd: allocated,
                        spent_usd: spent,
                        remaining_usd: (allocated - spent).max(0.0),
                        allocated_tokens: 0,
                        tokens_used: 0,
                        model_type: "local".to_string(),
                        status: if spent >= allocated {
                            "exhausted".to_string()
                        } else if allocated > 0.0 {
                            "active".to_string()
                        } else {
                            "passive".to_string()
                        },
                    });
                }

                (
                    config.limits.session_limit,
                    status.session_spent,
                    allocations,
                )
            } else {
                (None, arkavo_budget::TokenCost::ZERO, vec![])
            };

        let dashboard = ROICalculator::calculate_dashboard_with_allocations(
            &routing_metrics,
            &orch_metrics,
            session_limit,
            session_spent,
            agent_allocations,
        );

        Ok(dashboard)
    }

    async fn predict_workflow_cost(
        &self,
        _tasks: &[String],
    ) -> anyhow::Result<arkavo_router::WorkflowCostPrediction> {
        let orchestrator = self
            .orchestrator
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No orchestrator configured"))?;

        let orch_guard = orchestrator.read().await;
        let routing_metrics = orch_guard.get_routing_metrics().await;

        let predictor = WorkflowCostPredictor::new().with_history(vec![routing_metrics]);

        let prediction = predictor.predict(&[]);

        Ok(prediction)
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_budget::{BudgetConfig, BudgetManager};

    #[tokio::test]
    async fn test_cost_handler_creation() {
        let handler = CostHandler::new();
        assert!(handler.orchestrator.is_none());
    }

    #[tokio::test]
    async fn test_cost_metrics_without_orchestrator() {
        let handler = CostHandler::new();
        let result = handler.get_cost_metrics("24h").await;
        assert!(result.is_ok());
        let metrics = result.unwrap();
        assert_eq!(metrics.total_cost, 0.0);
        assert_eq!(metrics.time_range, "24h");
    }

    #[tokio::test]
    async fn test_cost_handler_with_orchestrator() {
        let config = BudgetConfig::default();
        let manager = BudgetManager::new(config).await.unwrap();
        let tracker = manager.tracker();

        let orchestrator = CostOrchestrator::new(tracker).await;
        if orchestrator.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        let orchestrator = orchestrator.unwrap();
        let (tx, _rx) = mpsc::channel(10);

        let _handler =
            CostHandler::new().with_orchestrator(Arc::new(RwLock::new(orchestrator)), tx);
    }
}
