use crate::config::{AlertType, BudgetAlert, BudgetConfig, BudgetThresholds};
use crate::cost::{TokenCost, TokenUsage};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use uuid::Uuid;

/// Metadata for architect mode cost tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectCostMetadata {
    /// Plan ID for grouping related costs
    pub plan_id: Uuid,
    /// Phase of architect workflow: "planning" or "execution"
    pub phase: String,
    /// Subtask index (0 for planning phase)
    pub subtask_index: Option<usize>,
    /// Subtask ID if in execution phase
    pub subtask_id: Option<Uuid>,
    /// What the cost would have been with Opus-only execution
    pub opus_only_estimate: f64,
}

/// Summary of cost savings from architect mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectSavingsReport {
    /// Plan ID
    pub plan_id: Uuid,
    /// Total actual cost incurred
    pub actual_cost: f64,
    /// What Opus-only would have cost
    pub opus_only_estimate: f64,
    /// Amount saved
    pub savings: f64,
    /// Savings as percentage
    pub savings_percent: f64,
    /// Number of subtasks executed
    pub subtask_count: usize,
    /// Cost by phase
    pub planning_cost: f64,
    pub execution_cost: f64,
}

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

    /// Atomically check affordability and record spending.
    /// Prevents race conditions by holding locks through both check and deduct.
    /// Returns Ok(SpendingRecord) if affordable, Err if budget would be exceeded.
    pub async fn try_spend(
        &self,
        agent_id: String,
        provider: String,
        model: String,
        usage: TokenUsage,
        cost: TokenCost,
    ) -> Result<SpendingRecord> {
        // Acquire config lock and hold through entire operation
        let mut config = self.config.write().await;
        config.time_windows.update();

        // Check global limits while holding lock
        {
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
                    && spent + cost > limit_value
                {
                    return Err(anyhow::anyhow!(
                        "Budget limit exceeded: {} + {} > {}",
                        spent,
                        cost,
                        limit_value
                    ));
                }
            }
        }

        // Check agent-specific limits
        if let Some(agent_budget) = config.agent_budgets.get(&agent_id) {
            let agent_statuses = self.agent_status.read().await;
            let agent_status = agent_statuses.get(&agent_id).cloned().unwrap_or_default();

            let agent_checks = [
                (agent_budget.session_limit, agent_status.session_spent),
                (agent_budget.hourly_limit, agent_status.hourly_spent),
                (agent_budget.daily_limit, agent_status.daily_spent),
            ];

            for (limit, spent) in agent_checks {
                if let Some(limit_value) = limit
                    && spent + cost > limit_value
                {
                    return Err(anyhow::anyhow!("Agent {} budget limit exceeded", agent_id));
                }
            }
        }

        // Checks passed - now record (still holding config lock)
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

        // Update global status
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

        // Update agent status
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

        // Record in history
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

    /// Record spending with architect mode metadata
    pub async fn record_architect_spending(
        &self,
        agent_id: String,
        provider: String,
        model: String,
        usage: TokenUsage,
        cost: TokenCost,
        architect_metadata: ArchitectCostMetadata,
    ) -> Result<SpendingRecord> {
        let mut metadata_map = HashMap::new();
        metadata_map.insert(
            "architect".to_string(),
            serde_json::to_value(&architect_metadata)?,
        );

        let record = SpendingRecord {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            agent_id: agent_id.clone(),
            provider,
            model,
            usage,
            cost,
            metadata: Some(metadata_map),
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

    /// Get cost savings report for a specific architect plan
    pub async fn get_architect_savings(&self, plan_id: Uuid) -> Option<ArchitectSavingsReport> {
        let history = self.spending_history.read().await;

        let mut actual_cost = 0.0;
        let mut opus_only_estimate = 0.0;
        let mut planning_cost = 0.0;
        let mut execution_cost = 0.0;
        let mut subtask_ids = std::collections::HashSet::new();

        for record in history.iter() {
            if let Some(metadata) = &record.metadata
                && let Some(architect_value) = metadata.get("architect")
                && let Ok(arch_meta) =
                    serde_json::from_value::<ArchitectCostMetadata>(architect_value.clone())
                && arch_meta.plan_id == plan_id
            {
                let cost_usd = record.cost.as_dollars();
                actual_cost += cost_usd;
                opus_only_estimate += arch_meta.opus_only_estimate;

                if arch_meta.phase == "planning" {
                    planning_cost += cost_usd;
                } else {
                    execution_cost += cost_usd;
                }

                if let Some(subtask_id) = arch_meta.subtask_id {
                    subtask_ids.insert(subtask_id);
                }
            }
        }

        if actual_cost == 0.0 {
            return None;
        }

        let savings = opus_only_estimate - actual_cost;
        let savings_percent = if opus_only_estimate > 0.0 {
            (savings / opus_only_estimate) * 100.0
        } else {
            0.0
        };

        Some(ArchitectSavingsReport {
            plan_id,
            actual_cost,
            opus_only_estimate,
            savings,
            savings_percent,
            subtask_count: subtask_ids.len(),
            planning_cost,
            execution_cost,
        })
    }

    /// Get all architect savings reports from the session
    pub async fn get_all_architect_savings(&self) -> Vec<ArchitectSavingsReport> {
        let history = self.spending_history.read().await;

        // Collect all unique plan IDs
        let mut plan_ids = std::collections::HashSet::new();
        for record in history.iter() {
            if let Some(metadata) = &record.metadata
                && let Some(architect_value) = metadata.get("architect")
                && let Ok(arch_meta) =
                    serde_json::from_value::<ArchitectCostMetadata>(architect_value.clone())
            {
                plan_ids.insert(arch_meta.plan_id);
            }
        }
        drop(history);

        // Generate reports for each plan
        let mut reports = Vec::new();
        for plan_id in plan_ids {
            if let Some(report) = self.get_architect_savings(plan_id).await {
                reports.push(report);
            }
        }

        reports
    }

    /// Get summary of architect mode usage
    pub async fn get_architect_summary(&self) -> ArchitectUsageSummary {
        let reports = self.get_all_architect_savings().await;

        let total_actual_cost: f64 = reports.iter().map(|r| r.actual_cost).sum();
        let total_opus_estimate: f64 = reports.iter().map(|r| r.opus_only_estimate).sum();
        let total_savings: f64 = reports.iter().map(|r| r.savings).sum();
        let total_subtasks: usize = reports.iter().map(|r| r.subtask_count).sum();

        ArchitectUsageSummary {
            plan_count: reports.len(),
            total_actual_cost,
            total_opus_estimate,
            total_savings,
            average_savings_percent: if !reports.is_empty() {
                reports.iter().map(|r| r.savings_percent).sum::<f64>() / reports.len() as f64
            } else {
                0.0
            },
            total_subtasks,
        }
    }
}

/// Summary of all architect mode usage in the session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectUsageSummary {
    /// Number of architect plans executed
    pub plan_count: usize,
    /// Total actual cost
    pub total_actual_cost: f64,
    /// Total estimated cost if using Opus only
    pub total_opus_estimate: f64,
    /// Total savings
    pub total_savings: f64,
    /// Average savings percentage across all plans
    pub average_savings_percent: f64,
    /// Total subtasks executed
    pub total_subtasks: usize,
}

#[cfg(test)]
#[path = "tracker_tests.rs"]
mod tracker_tests;
