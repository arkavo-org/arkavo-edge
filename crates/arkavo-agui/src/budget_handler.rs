use crate::types::AgUiEvent;
use arkavo_budget::tracker::BudgetStatus;
use arkavo_budget::{BudgetConfig, BudgetManager};
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct BudgetHandler {
    manager: Option<Arc<BudgetManager>>,
    event_tx: Option<mpsc::Sender<AgUiEvent>>,
}

impl Default for BudgetHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl BudgetHandler {
    pub fn new() -> Self {
        Self {
            manager: None,
            event_tx: None,
        }
    }

    pub async fn initialize(
        &mut self,
        config: BudgetConfig,
        event_tx: mpsc::Sender<AgUiEvent>,
    ) -> anyhow::Result<()> {
        let manager = Arc::new(BudgetManager::new(config).await?);
        self.manager = Some(manager.clone());
        self.event_tx = Some(event_tx.clone());

        // Start forwarding budget events to AGUI
        self.start_event_forwarding().await;

        Ok(())
    }

    async fn start_event_forwarding(&self) {
        if let (Some(manager), Some(event_tx)) = (&self.manager, &self.event_tx) {
            let tracker = manager.tracker();
            let mut budget_rx = tracker.subscribe_events();
            let event_tx = event_tx.clone();

            tokio::spawn(async move {
                while let Ok(event) = budget_rx.recv().await {
                    use arkavo_budget::tracker::BudgetEvent;

                    let agui_event = match event {
                        BudgetEvent::SpendingRecorded(record) => {
                            Some(AgUiEvent::SpendingRecorded {
                                record,
                                event_id: uuid::Uuid::new_v4().to_string(),
                            })
                        }
                        BudgetEvent::ThresholdExceeded(alert)
                        | BudgetEvent::BudgetExhausted(alert) => Some(AgUiEvent::BudgetAlert {
                            alert,
                            event_id: uuid::Uuid::new_v4().to_string(),
                        }),
                        BudgetEvent::BudgetReset { .. } => None,
                    };

                    if let Some(event) = agui_event {
                        let _ = event_tx.send(event).await;
                    }
                }
            });
        }
    }

    pub async fn handle_event(
        &self,
        event: &AgUiEvent,
        tx: &mpsc::Sender<AgUiEvent>,
    ) -> anyhow::Result<()> {
        if let Some(manager) = &self.manager {
            match event {
                AgUiEvent::GetBudgetStatus { agent_id } => {
                    let tracker = manager.tracker();

                    let status = if let Some(id) = agent_id.as_ref() {
                        tracker.get_agent_status(id).await.unwrap_or_default()
                    } else {
                        let status_with_limits = tracker.get_status().await;
                        BudgetStatus {
                            session_spent: status_with_limits.session_spent,
                            hourly_spent: status_with_limits.hourly_spent,
                            daily_spent: status_with_limits.daily_spent,
                            monthly_spent: status_with_limits.monthly_spent,
                            total_spent: status_with_limits.total_spent,
                            last_updated: status_with_limits.last_updated,
                        }
                    };

                    let response = AgUiEvent::BudgetStatusUpdate {
                        agent_id: agent_id.clone(),
                        status,
                        event_id: uuid::Uuid::new_v4().to_string(),
                    };

                    tx.send(response).await?;
                }

                AgUiEvent::SetAgentBudget { agent_id, budget } => {
                    let mut config = manager.get_config().await;
                    config
                        .agent_budgets
                        .insert(agent_id.clone(), budget.clone());
                    manager.update_config(config.clone()).await?;

                    let response = AgUiEvent::BudgetConfigUpdate {
                        config,
                        event_id: uuid::Uuid::new_v4().to_string(),
                    };

                    tx.send(response).await?;
                }

                AgUiEvent::ResetBudgetWindow { window } => {
                    let tracker = manager.tracker();
                    tracker.reset_time_window(window).await?;
                }

                _ => {}
            }
        }

        Ok(())
    }

    pub fn manager(&self) -> Option<Arc<BudgetManager>> {
        self.manager.clone()
    }

    pub async fn record_spending(
        &self,
        agent_id: String,
        provider: String,
        model: String,
        usage: arkavo_budget::cost::TokenUsage,
        cost: arkavo_budget::TokenCost,
    ) -> anyhow::Result<()> {
        if let Some(manager) = &self.manager {
            let tracker = manager.tracker();
            tracker
                .record_spending(agent_id, provider, model, usage, cost)
                .await?;
        }
        Ok(())
    }

    pub async fn can_afford(
        &self,
        agent_id: &str,
        estimated_cost: arkavo_budget::TokenCost,
    ) -> anyhow::Result<bool> {
        if let Some(manager) = &self.manager {
            let tracker = manager.tracker();
            return tracker.can_afford(agent_id, estimated_cost).await;
        }
        Ok(true) // If no budget manager, allow all requests
    }
}
