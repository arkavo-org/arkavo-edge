use crate::config::{AgentBudget, BudgetAlert, BudgetConfig};
use crate::cost::TokenCost;
use crate::tracker::{BudgetEvent, BudgetStatus, SpendingRecord};
use crate::{BudgetManager, SelectionCriteria};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Budget-related events that can be sent via AGUI WebSocket
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BudgetAgUiEvent {
    // Budget status updates
    BudgetStatusUpdate {
        #[serde(rename = "agentId")]
        agent_id: Option<String>,
        status: BudgetStatus,
        #[serde(rename = "eventId")]
        event_id: String,
    },

    // Budget alert
    BudgetAlert {
        alert: BudgetAlert,
        #[serde(rename = "eventId")]
        event_id: String,
    },

    // Spending record
    SpendingRecorded {
        record: SpendingRecord,
        #[serde(rename = "eventId")]
        event_id: String,
    },

    // Budget configuration update
    BudgetConfigUpdate {
        config: BudgetConfig,
        #[serde(rename = "eventId")]
        event_id: String,
    },

    // Model selection event
    ModelSelected {
        #[serde(rename = "agentId")]
        agent_id: String,
        provider: String,
        model: String,
        #[serde(rename = "estimatedCost")]
        estimated_cost: TokenCost,
        reason: String,
        #[serde(rename = "eventId")]
        event_id: String,
    },

    // Budget request/response for UI control
    GetBudgetStatus {
        #[serde(rename = "agentId")]
        agent_id: Option<String>,
    },

    SetAgentBudget {
        #[serde(rename = "agentId")]
        agent_id: String,
        budget: AgentBudget,
    },

    ResetBudgetWindow {
        window: String,
    },
}

/// Budget API handler for AGUI integration
pub struct BudgetApiHandler {
    manager: Arc<BudgetManager>,
    event_tx: mpsc::Sender<BudgetAgUiEvent>,
}

impl BudgetApiHandler {
    pub fn new(manager: Arc<BudgetManager>, event_tx: mpsc::Sender<BudgetAgUiEvent>) -> Self {
        Self { manager, event_tx }
    }

    /// Start listening to budget tracker events and forward them to AGUI
    pub async fn start_event_forwarding(&self) {
        let tracker = self.manager.tracker();
        let mut event_rx = tracker.subscribe_events();
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            while let Ok(event) = event_rx.recv().await {
                let agui_event = match event {
                    BudgetEvent::SpendingRecorded(record) => {
                        Some(BudgetAgUiEvent::SpendingRecorded {
                            record,
                            event_id: uuid::Uuid::new_v4().to_string(),
                        })
                    }
                    BudgetEvent::ThresholdExceeded(alert) | BudgetEvent::BudgetExhausted(alert) => {
                        Some(BudgetAgUiEvent::BudgetAlert {
                            alert,
                            event_id: uuid::Uuid::new_v4().to_string(),
                        })
                    }
                    BudgetEvent::BudgetReset { .. } => None,
                };

                if let Some(event) = agui_event {
                    let _ = event_tx.send(event).await;
                }
            }
        });
    }

    /// Handle incoming budget-related AGUI events
    pub async fn handle_event(&self, event: BudgetAgUiEvent) -> Result<Option<BudgetAgUiEvent>> {
        match event {
            BudgetAgUiEvent::GetBudgetStatus { agent_id } => {
                let tracker = self.manager.tracker();

                let status = if let Some(id) = agent_id {
                    tracker.get_agent_status(&id).await.unwrap_or_default()
                } else {
                    tracker.get_status().await
                };

                Ok(Some(BudgetAgUiEvent::BudgetStatusUpdate {
                    agent_id: None,
                    status,
                    event_id: uuid::Uuid::new_v4().to_string(),
                }))
            }

            BudgetAgUiEvent::SetAgentBudget { agent_id, budget } => {
                let mut config = self.manager.get_config().await;
                config.agent_budgets.insert(agent_id, budget);
                self.manager.update_config(config.clone()).await?;

                Ok(Some(BudgetAgUiEvent::BudgetConfigUpdate {
                    config,
                    event_id: uuid::Uuid::new_v4().to_string(),
                }))
            }

            BudgetAgUiEvent::ResetBudgetWindow { window } => {
                let tracker = self.manager.tracker();
                tracker.reset_time_window(&window).await?;
                Ok(None)
            }

            _ => Ok(None),
        }
    }
}

/// Budget status response format for REST API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetStatusResponse {
    pub global: BudgetStatus,
    pub agents: Vec<AgentBudgetStatus>,
    pub recent_spending: Vec<SpendingRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBudgetStatus {
    pub agent_id: String,
    pub status: BudgetStatus,
    pub budget: Option<AgentBudget>,
}

/// Budget breakdown response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetBreakdown {
    pub by_provider: Vec<ProviderSpending>,
    pub by_model: Vec<ModelSpending>,
    pub by_agent: Vec<AgentSpending>,
    pub time_period: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSpending {
    pub provider: String,
    pub total_cost: TokenCost,
    pub request_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpending {
    pub provider: String,
    pub model: String,
    pub total_cost: TokenCost,
    pub request_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpending {
    pub agent_id: String,
    pub total_cost: TokenCost,
    pub request_count: u32,
}

/// Model selection request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSelectionRequest {
    pub agent_id: String,
    pub criteria: SelectionCriteria,
    pub available_models: Vec<String>,
}

/// Model selection response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSelectionResponse {
    pub selected_model: Option<ModelSelection>,
    pub alternatives: Vec<ModelSelection>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSelection {
    pub provider: String,
    pub model_id: String,
    pub estimated_cost: TokenCost,
    pub score: f64,
}
