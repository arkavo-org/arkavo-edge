use crate::cost::TokenCost;
use chrono::{DateTime, Datelike, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BudgetConfig {
    pub limits: BudgetLimits,
    pub thresholds: BudgetThresholds,
    pub time_windows: TimeWindows,
    pub agent_budgets: HashMap<String, AgentBudget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetLimits {
    pub session_limit: Option<TokenCost>,
    pub hourly_limit: Option<TokenCost>,
    pub daily_limit: Option<TokenCost>,
    pub monthly_limit: Option<TokenCost>,
    pub total_limit: Option<TokenCost>,
}

impl Default for BudgetLimits {
    fn default() -> Self {
        Self {
            session_limit: Some(TokenCost::from_dollars(10.0)),
            hourly_limit: Some(TokenCost::from_dollars(5.0)),
            daily_limit: Some(TokenCost::from_dollars(50.0)),
            monthly_limit: Some(TokenCost::from_dollars(500.0)),
            total_limit: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetThresholds {
    pub warning_percent: u8,
    pub critical_percent: u8,
    pub emergency_percent: u8,
}

impl Default for BudgetThresholds {
    fn default() -> Self {
        Self {
            warning_percent: 50,
            critical_percent: 80,
            emergency_percent: 95,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeWindows {
    pub session_start: DateTime<Utc>,
    pub hour_start: DateTime<Utc>,
    pub day_start: DateTime<Utc>,
    pub month_start: DateTime<Utc>,
}

impl Default for TimeWindows {
    fn default() -> Self {
        let now = Utc::now();
        Self {
            session_start: now,
            hour_start: now,
            day_start: now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc(),
            month_start: now
                .date_naive()
                .with_day(1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc(),
        }
    }
}

impl TimeWindows {
    pub fn update(&mut self) {
        let now = Utc::now();

        if now - self.hour_start >= Duration::hours(1) {
            self.hour_start = now;
        }

        let today_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
        if today_start > self.day_start {
            self.day_start = today_start;
        }

        let month_start = now
            .date_naive()
            .with_day(1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        if month_start > self.month_start {
            self.month_start = month_start;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBudget {
    pub agent_id: String,
    pub session_limit: Option<TokenCost>,
    pub hourly_limit: Option<TokenCost>,
    pub daily_limit: Option<TokenCost>,
    pub priority: BudgetPriority,
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum BudgetPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl AgentBudget {
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            session_limit: None,
            hourly_limit: None,
            daily_limit: None,
            priority: BudgetPriority::Normal,
            metadata: HashMap::new(),
        }
    }

    pub fn with_session_limit(mut self, limit: TokenCost) -> Self {
        self.session_limit = Some(limit);
        self
    }

    pub fn with_priority(mut self, priority: BudgetPriority) -> Self {
        self.priority = priority;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetAlert {
    pub timestamp: DateTime<Utc>,
    pub alert_type: AlertType,
    pub message: String,
    pub agent_id: Option<String>,
    pub current_spend: TokenCost,
    pub limit: TokenCost,
    pub percent_used: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertType {
    Warning,
    Critical,
    Emergency,
    Exhausted,
}

impl BudgetAlert {
    pub fn new(
        alert_type: AlertType,
        message: String,
        current_spend: TokenCost,
        limit: TokenCost,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            alert_type,
            message,
            agent_id: None,
            current_spend,
            limit,
            percent_used: (current_spend.as_cents() as f64 / limit.as_cents() as f64) * 100.0,
        }
    }

    pub fn for_agent(mut self, agent_id: String) -> Self {
        self.agent_id = Some(agent_id);
        self
    }
}
