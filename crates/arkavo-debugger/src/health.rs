use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub status: HealthStatus,
    pub agent_id: String,
    pub session_id: String,
    pub uptime_seconds: u64,
    pub total_events: u64,
    pub total_sessions: u64,
    pub error_rate: f64,
    pub last_error: Option<LastError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastError {
    pub timestamp: DateTime<Utc>,
    pub error_type: String,
    pub message: String,
}

impl HealthReport {
    pub fn is_healthy(&self) -> bool {
        self.status == HealthStatus::Healthy
    }

    pub fn calculate_status(error_rate: f64, recent_errors: u32) -> HealthStatus {
        if error_rate > 0.1 || recent_errors > 10 {
            HealthStatus::Unhealthy
        } else if error_rate > 0.05 || recent_errors > 5 {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        }
    }
}
