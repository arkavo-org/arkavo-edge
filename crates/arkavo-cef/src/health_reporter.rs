use crate::command_tracker::{CommandHealthReport, CommandTracker};
use arkavo_observability::health_reporter::{HealthReport, HealthReporter};
use async_trait::async_trait;
use std::sync::Arc;

pub struct CefHealthReporter {
    tracker: Arc<CommandTracker>,
}

impl CefHealthReporter {
    pub fn new(tracker: Arc<CommandTracker>) -> Self {
        Self { tracker }
    }
}

#[async_trait]
impl HealthReporter for CefHealthReporter {
    async fn check_health(&self) -> HealthReport {
        let snapshot = self.tracker.get_health_snapshot().await;

        let stuck_commands: Vec<&CommandHealthReport> = snapshot
            .commands
            .iter()
            .filter(|cmd| cmd.duration_ms > 5000)
            .collect();

        if !stuck_commands.is_empty() {
            let details = stuck_commands
                .iter()
                .map(|cmd| {
                    format!(
                        "Command {} ({}) waiting {}ms on '{}'",
                        cmd.id, cmd.op_name, cmd.duration_ms, cmd.selector
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");

            HealthReport::degraded("cef", format!("Stuck commands: {details}"))
        } else if !snapshot.commands.is_empty() {
            HealthReport::healthy(
                "cef",
                format!("{} active commands", snapshot.commands.len()),
            )
        } else {
            HealthReport::healthy("cef", "No active commands")
        }
    }

    fn component_name(&self) -> &'static str {
        "cef"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_reporter_no_commands() {
        let tracker = Arc::new(CommandTracker::new());
        let reporter = CefHealthReporter::new(tracker);

        let health = reporter.check_health().await;
        assert_eq!(health.status, HealthStatus::Healthy);
        assert_eq!(health.message, "No active commands");
    }

    #[tokio::test]
    async fn test_health_reporter_active_commands() {
        let tracker = Arc::new(CommandTracker::new());
        tracker.track_command(1, 0, "#test".to_string()).await;

        let reporter = CefHealthReporter::new(tracker);
        let health = reporter.check_health().await;

        assert_eq!(health.status, HealthStatus::Healthy);
        assert!(health.message.contains("1 active commands"));
    }

    #[tokio::test]
    async fn test_health_reporter_stuck_commands() {
        let tracker = Arc::new(CommandTracker::new());
        tracker.track_command(1, 0, "#test".to_string()).await;

        tokio::time::sleep(tokio::time::Duration::from_millis(5100)).await;

        let reporter = CefHealthReporter::new(tracker);
        let health = reporter.check_health().await;

        assert_eq!(health.status, HealthStatus::Degraded);
        assert!(health.message.contains("Stuck commands"));
        assert!(health.message.contains("Command 1"));
    }
}
