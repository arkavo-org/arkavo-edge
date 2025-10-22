use crate::config::{AlertType, BudgetAlert, BudgetConfig, BudgetThresholds};
use crate::cost::{TokenCost, TokenUsage};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpendingRecord {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub agent_id: String,
    pub provider: String,
    pub model: String,
    pub usage: TokenUsage,
    pub cost: TokenCost,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetStatus {
    pub session_spent: TokenCost,
    pub hourly_spent: TokenCost,
    pub daily_spent: TokenCost,
    pub monthly_spent: TokenCost,
    pub total_spent: TokenCost,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetStatusWithLimits {
    pub session_spent: TokenCost,
    pub hourly_spent: TokenCost,
    pub daily_spent: TokenCost,
    pub monthly_spent: TokenCost,
    pub total_spent: TokenCost,
    pub last_updated: DateTime<Utc>,
    pub session_limit: Option<TokenCost>,
    pub session_remaining: Option<TokenCost>,
    pub session_usage_percent: f64,
}

impl Default for BudgetStatus {
    fn default() -> Self {
        Self {
            session_spent: TokenCost::ZERO,
            hourly_spent: TokenCost::ZERO,
            daily_spent: TokenCost::ZERO,
            monthly_spent: TokenCost::ZERO,
            total_spent: TokenCost::ZERO,
            last_updated: Utc::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum BudgetEvent {
    SpendingRecorded(SpendingRecord),
    ThresholdExceeded(BudgetAlert),
    BudgetExhausted(BudgetAlert),
    BudgetReset { window: String },
}

pub struct BudgetTracker {
    config: Arc<RwLock<BudgetConfig>>,
    status: Arc<RwLock<BudgetStatus>>,
    agent_status: Arc<RwLock<HashMap<String, BudgetStatus>>>,
    spending_history: Arc<RwLock<Vec<SpendingRecord>>>,
    event_sender: broadcast::Sender<BudgetEvent>,
}

impl BudgetTracker {
    pub async fn new(config: BudgetConfig) -> Result<Self> {
        let (event_sender, _) = broadcast::channel(1000);

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            status: Arc::new(RwLock::new(BudgetStatus::default())),
            agent_status: Arc::new(RwLock::new(HashMap::new())),
            spending_history: Arc::new(RwLock::new(Vec::new())),
            event_sender,
        })
    }

    pub async fn can_afford(&self, agent_id: &str, estimated_cost: TokenCost) -> Result<bool> {
        let config = self.config.read().await;
        let status = self.status.read().await;

        let global_checks = [
            (config.limits.session_limit, status.session_spent),
            (config.limits.hourly_limit, status.hourly_spent),
            (config.limits.daily_limit, status.daily_spent),
            (config.limits.monthly_limit, status.monthly_spent),
            (config.limits.total_limit, status.total_spent),
        ];

        for (limit, spent) in global_checks {
            if let Some(limit_value) = limit
                && spent + estimated_cost > limit_value
            {
                return Ok(false);
            }
        }

        if let Some(agent_budget) = config.agent_budgets.get(agent_id) {
            let agent_statuses = self.agent_status.read().await;
            let agent_status = agent_statuses.get(agent_id).cloned().unwrap_or_default();

            let agent_checks = [
                (agent_budget.session_limit, agent_status.session_spent),
                (agent_budget.hourly_limit, agent_status.hourly_spent),
                (agent_budget.daily_limit, agent_status.daily_spent),
            ];

            for (limit, spent) in agent_checks {
                if let Some(limit_value) = limit
                    && spent + estimated_cost > limit_value
                {
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    pub async fn record_spending(
        &self,
        agent_id: String,
        provider: String,
        model: String,
        usage: TokenUsage,
        cost: TokenCost,
    ) -> Result<SpendingRecord> {
        let record = SpendingRecord {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            agent_id: agent_id.clone(),
            provider,
            model,
            usage,
            cost,
            metadata: None,
        };

        let mut config = self.config.write().await;
        config.time_windows.update();

        {
            let mut status = self.status.write().await;
            status.session_spent += cost;
            status.hourly_spent += cost;
            status.daily_spent += cost;
            status.monthly_spent += cost;
            status.total_spent += cost;
            status.last_updated = Utc::now();

            self.check_thresholds(&config, &status, None).await?;
        }

        {
            let mut agent_statuses = self.agent_status.write().await;
            let agent_status = agent_statuses
                .entry(agent_id.clone())
                .or_insert_with(BudgetStatus::default);

            agent_status.session_spent += cost;
            agent_status.hourly_spent += cost;
            agent_status.daily_spent += cost;
            agent_status.monthly_spent += cost;
            agent_status.total_spent += cost;
            agent_status.last_updated = Utc::now();

            if let Some(agent_budget) = config.agent_budgets.get(&agent_id) {
                self.check_agent_thresholds(&config.thresholds, agent_budget, agent_status)
                    .await?;
            }
        }

        {
            let mut history = self.spending_history.write().await;
            history.push(record.clone());

            if history.len() > 10000 {
                history.drain(0..1000);
            }
        }

        let _ = self
            .event_sender
            .send(BudgetEvent::SpendingRecorded(record.clone()));

        Ok(record)
    }

    async fn check_thresholds(
        &self,
        config: &BudgetConfig,
        status: &BudgetStatus,
        agent_id: Option<String>,
    ) -> Result<()> {
        let checks = [
            ("session", config.limits.session_limit, status.session_spent),
            ("hourly", config.limits.hourly_limit, status.hourly_spent),
            ("daily", config.limits.daily_limit, status.daily_spent),
            ("monthly", config.limits.monthly_limit, status.monthly_spent),
            ("total", config.limits.total_limit, status.total_spent),
        ];

        for (window, limit_opt, spent) in checks {
            if let Some(limit) = limit_opt {
                let percent = (spent.as_cents() as f64 / limit.as_cents() as f64) * 100.0;

                let alert_type = if percent >= 100.0 {
                    Some(AlertType::Exhausted)
                } else if percent >= config.thresholds.emergency_percent as f64 {
                    Some(AlertType::Emergency)
                } else if percent >= config.thresholds.critical_percent as f64 {
                    Some(AlertType::Critical)
                } else if percent >= config.thresholds.warning_percent as f64 {
                    Some(AlertType::Warning)
                } else {
                    None
                };

                if let Some(alert_type) = alert_type {
                    let message = format!(
                        "{} budget {} at {:.1}% ({} of {})",
                        window,
                        match alert_type {
                            AlertType::Exhausted => "exhausted",
                            AlertType::Emergency => "emergency threshold",
                            AlertType::Critical => "critical threshold",
                            AlertType::Warning => "warning threshold",
                        },
                        percent,
                        spent,
                        limit
                    );

                    let mut alert = BudgetAlert::new(alert_type, message, spent, limit);
                    if let Some(id) = agent_id.clone() {
                        alert = alert.for_agent(id);
                    }

                    let event = match alert_type {
                        AlertType::Exhausted => BudgetEvent::BudgetExhausted(alert),
                        _ => BudgetEvent::ThresholdExceeded(alert),
                    };

                    let _ = self.event_sender.send(event);
                }
            }
        }

        Ok(())
    }

    async fn check_agent_thresholds(
        &self,
        thresholds: &BudgetThresholds,
        agent_budget: &crate::config::AgentBudget,
        agent_status: &BudgetStatus,
    ) -> Result<()> {
        let checks = [
            (
                "session",
                agent_budget.session_limit,
                agent_status.session_spent,
            ),
            (
                "hourly",
                agent_budget.hourly_limit,
                agent_status.hourly_spent,
            ),
            ("daily", agent_budget.daily_limit, agent_status.daily_spent),
        ];

        for (window, limit_opt, spent) in checks {
            if let Some(limit) = limit_opt {
                let percent = (spent.as_cents() as f64 / limit.as_cents() as f64) * 100.0;

                let alert_type = if percent >= 100.0 {
                    Some(AlertType::Exhausted)
                } else if percent >= thresholds.emergency_percent as f64 {
                    Some(AlertType::Emergency)
                } else if percent >= thresholds.critical_percent as f64 {
                    Some(AlertType::Critical)
                } else if percent >= thresholds.warning_percent as f64 {
                    Some(AlertType::Warning)
                } else {
                    None
                };

                if let Some(alert_type) = alert_type {
                    let message = format!(
                        "Agent {} {} budget {} at {:.1}%",
                        agent_budget.agent_id,
                        window,
                        match alert_type {
                            AlertType::Exhausted => "exhausted",
                            AlertType::Emergency => "emergency",
                            AlertType::Critical => "critical",
                            AlertType::Warning => "warning",
                        },
                        percent
                    );

                    let alert = BudgetAlert::new(alert_type, message, spent, limit)
                        .for_agent(agent_budget.agent_id.clone());

                    let event = match alert_type {
                        AlertType::Exhausted => BudgetEvent::BudgetExhausted(alert),
                        _ => BudgetEvent::ThresholdExceeded(alert),
                    };

                    let _ = self.event_sender.send(event);
                }
            }
        }

        Ok(())
    }

    pub async fn get_status(&self) -> BudgetStatusWithLimits {
        let status = self.status.read().await.clone();
        let config = self.config.read().await;

        BudgetStatusWithLimits {
            session_spent: status.session_spent,
            hourly_spent: status.hourly_spent,
            daily_spent: status.daily_spent,
            monthly_spent: status.monthly_spent,
            total_spent: status.total_spent,
            last_updated: status.last_updated,
            session_limit: config.limits.session_limit,
            session_remaining: config
                .limits
                .session_limit
                .map(|l| l - status.session_spent),
            session_usage_percent: config
                .limits
                .session_limit
                .map(|l| (status.session_spent.as_cents() as f64 / l.as_cents() as f64) * 100.0)
                .unwrap_or(0.0),
        }
    }

    pub async fn get_agent_status(&self, agent_id: &str) -> Option<BudgetStatus> {
        self.agent_status.read().await.get(agent_id).cloned()
    }

    pub async fn get_spending_history(&self, limit: usize) -> Vec<SpendingRecord> {
        let history = self.spending_history.read().await;
        history.iter().rev().take(limit).cloned().collect()
    }

    pub async fn reset_time_window(&self, window: &str) -> Result<()> {
        let mut status = self.status.write().await;

        match window {
            "session" => status.session_spent = TokenCost::ZERO,
            "hourly" => status.hourly_spent = TokenCost::ZERO,
            "daily" => status.daily_spent = TokenCost::ZERO,
            "monthly" => status.monthly_spent = TokenCost::ZERO,
            _ => return Err(anyhow::anyhow!("Invalid time window: {}", window)),
        }

        let _ = self.event_sender.send(BudgetEvent::BudgetReset {
            window: window.to_string(),
        });

        Ok(())
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<BudgetEvent> {
        self.event_sender.subscribe()
    }
}

#[cfg(test)]
#[path = "tracker_tests.rs"]
mod tracker_tests;
