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

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn sample_report(status: HealthStatus) -> HealthReport {
        HealthReport {
            status,
            agent_id: "agent-1".to_string(),
            session_id: "session-1".to_string(),
            uptime_seconds: 120,
            total_events: 42,
            total_sessions: 3,
            error_rate: 0.02,
            last_error: Some(LastError {
                timestamp: Utc::now(),
                error_type: "timeout".to_string(),
                message: "request timed out".to_string(),
            }),
        }
    }

    #[spec("DBG-004")]
    #[test]
    fn test_health_report_aggregation_and_format() {
        let report = sample_report(HealthStatus::Healthy);

        // Aggregated metrics are present on the report
        assert_eq!(report.total_events, 42);
        assert_eq!(report.total_sessions, 3);
        assert!(report.is_healthy());

        // Report is formatted as structured JSON with snake_case status
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["status"], "healthy");
        assert_eq!(json["agent_id"], "agent-1");
        assert_eq!(json["last_error"]["error_type"], "timeout");

        let roundtrip: HealthReport = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.status, HealthStatus::Healthy);
        assert_eq!(roundtrip.total_events, 42);
    }

    #[spec("DBG-005")]
    #[test]
    fn test_calculate_status_healthy() {
        assert_eq!(
            HealthReport::calculate_status(0.0, 0),
            HealthStatus::Healthy
        );
        assert_eq!(
            HealthReport::calculate_status(0.05, 5),
            HealthStatus::Healthy
        );
    }

    #[spec("DBG-005")]
    #[test]
    fn test_calculate_status_degraded_and_failed() {
        // Subsystem degraded: elevated error rate or too many recent errors
        assert_eq!(
            HealthReport::calculate_status(0.06, 0),
            HealthStatus::Degraded
        );
        assert_eq!(
            HealthReport::calculate_status(0.0, 6),
            HealthStatus::Degraded
        );

        // Critical failure: high error rate or excessive recent errors
        assert_eq!(
            HealthReport::calculate_status(0.11, 0),
            HealthStatus::Unhealthy
        );
        assert_eq!(
            HealthReport::calculate_status(0.0, 11),
            HealthStatus::Unhealthy
        );
    }
}
