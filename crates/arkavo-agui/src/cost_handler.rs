use crate::roi_metrics::{CostMetrics, HourlyCost, ROICalculator, ROIDashboard};
use crate::types::AgUiEvent;
use arkavo_router::{CostOrchestrator, WorkflowCostPredictor};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

pub struct CostHandler {
    orchestrator: Option<Arc<RwLock<CostOrchestrator>>>,
    event_tx: Option<mpsc::Sender<AgUiEvent>>,
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
        }
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
        let orchestrator = self
            .orchestrator
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No orchestrator configured"))?;

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
        let orchestrator = self
            .orchestrator
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No orchestrator configured"))?;

        let orch_guard = orchestrator.read().await;
        let routing_metrics = orch_guard.get_routing_metrics().await;
        let orch_metrics = orch_guard.get_orchestrator_metrics().await;

        let dashboard = ROICalculator::calculate_dashboard(
            &routing_metrics,
            &orch_metrics,
            None,
            arkavo_budget::TokenCost::ZERO,
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
        assert!(result.is_err());
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
