use crate::command_health_collector::{HealthBatch, TimeoutAnalysis, TimeoutAnalyzer};
use crate::types::{AgUiEvent, NotificationSeverity};
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;

pub struct TimeoutHandler {
    analyzer: Arc<TimeoutAnalyzer>,
}

impl TimeoutHandler {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            analyzer: Arc::new(TimeoutAnalyzer::new().await?),
        })
    }

    #[cfg(test)]
    pub async fn new_offline() -> Result<Self> {
        Ok(Self {
            analyzer: Arc::new(TimeoutAnalyzer::new_offline().await?),
        })
    }

    pub async fn start(
        self,
        mut batch_rx: mpsc::Receiver<HealthBatch>,
        event_tx: mpsc::Sender<AgUiEvent>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(batch) = batch_rx.recv().await {
                if let Err(e) = self.process_batch(batch, &event_tx).await {
                    eprintln!("Timeout handler error: {e}");
                }
            }
        })
    }

    async fn process_batch(
        &self,
        batch: HealthBatch,
        event_tx: &mpsc::Sender<AgUiEvent>,
    ) -> Result<()> {
        let analyses = self.analyzer.analyze_batch(batch).await?;
        publish_analyses(&analyses, event_tx).await;
        Ok(())
    }
}

/// Notifications go out before the summary so consumers see the details that
/// explain the summary's unhealthy count.
async fn publish_analyses(analyses: &[TimeoutAnalysis], event_tx: &mpsc::Sender<AgUiEvent>) {
    let total = analyses.len();
    let mut unhealthy = 0;
    for analysis in analyses {
        if analysis.should_timeout {
            unhealthy += 1;
            if let Some(ref msg) = analysis.user_message {
                send_notification(msg.clone(), &analysis.severity, event_tx).await;
            }
        }
    }

    // Emit health summary so the telemetry stream always has health visibility
    let summary = AgUiEvent::TelemetryEvent {
        event_type: "health_summary".to_string(),
        agent_id: "system".to_string(),
        details: serde_json::json!({
            "healthy": total - unhealthy,
            "unhealthy": unhealthy,
            "total": total,
        }),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    let _ = event_tx.send(summary).await;
}

async fn send_notification(message: String, severity: &str, event_tx: &mpsc::Sender<AgUiEvent>) {
    let event = AgUiEvent::SystemNotification {
        message,
        severity: match severity {
            "critical" => NotificationSeverity::Error,
            "warning" => NotificationSeverity::Warning,
            _ => NotificationSeverity::Info,
        },
    };

    if let Err(e) = event_tx.send(event).await {
        eprintln!("Failed to send timeout notification: {e}");
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::command_health_collector::CommandHealthData;

    fn analysis(should_timeout: bool, message: Option<&str>, severity: &str) -> TimeoutAnalysis {
        TimeoutAnalysis {
            should_timeout,
            user_message: message.map(str::to_string),
            severity: severity.to_string(),
            reasoning: String::new(),
        }
    }

    async fn drain(event_rx: &mut mpsc::Receiver<AgUiEvent>) -> Vec<AgUiEvent> {
        let mut events = Vec::new();
        while let Ok(Some(event)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), event_rx.recv()).await
        {
            events.push(event);
        }
        events
    }

    #[tokio::test]
    async fn timed_out_command_notifies_before_health_summary() {
        let (event_tx, mut event_rx) = mpsc::channel(10);

        publish_analyses(
            &[analysis(true, Some("Command stuck"), "critical")],
            &event_tx,
        )
        .await;
        drop(event_tx);

        let events = drain(&mut event_rx).await;
        assert_eq!(events.len(), 2, "expected notification then summary");
        match &events[0] {
            AgUiEvent::SystemNotification { message, severity } => {
                assert_eq!(message, "Command stuck");
                assert!(matches!(severity, NotificationSeverity::Error));
            }
            other => panic!("Expected SystemNotification first, got {other:?}"),
        }
        match &events[1] {
            AgUiEvent::TelemetryEvent {
                event_type,
                details,
                ..
            } => {
                assert_eq!(event_type, "health_summary");
                assert_eq!(details["total"], 1);
                assert_eq!(details["unhealthy"], 1);
                assert_eq!(details["healthy"], 0);
            }
            other => panic!("Expected health_summary second, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn healthy_command_emits_only_health_summary() {
        let (event_tx, mut event_rx) = mpsc::channel(10);

        publish_analyses(
            &[analysis(false, Some("Running normally"), "info")],
            &event_tx,
        )
        .await;
        drop(event_tx);

        let events = drain(&mut event_rx).await;
        assert_eq!(events.len(), 1, "healthy batch must not notify");
        match &events[0] {
            AgUiEvent::TelemetryEvent {
                event_type,
                details,
                ..
            } => {
                assert_eq!(event_type, "health_summary");
                assert_eq!(details["healthy"], 1);
                assert_eq!(details["unhealthy"], 0);
            }
            other => panic!("Expected health_summary, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn severity_maps_to_notification_levels() {
        let (event_tx, mut event_rx) = mpsc::channel(10);

        publish_analyses(
            &[
                analysis(true, Some("w"), "warning"),
                analysis(true, Some("i"), "info"),
                analysis(true, Some("b"), "bogus"),
            ],
            &event_tx,
        )
        .await;
        drop(event_tx);

        let events = drain(&mut event_rx).await;
        assert_eq!(events.len(), 4, "expected 3 notifications then summary");
        assert!(matches!(
            events[0],
            AgUiEvent::SystemNotification {
                severity: NotificationSeverity::Warning,
                ..
            }
        ));
        assert!(matches!(
            events[1],
            AgUiEvent::SystemNotification {
                severity: NotificationSeverity::Info,
                ..
            }
        ));
        assert!(
            matches!(
                events[2],
                AgUiEvent::SystemNotification {
                    severity: NotificationSeverity::Info,
                    ..
                }
            ),
            "unknown severity must fall back to Info"
        );
    }

    #[tokio::test]
    async fn timeout_without_message_counts_unhealthy_but_does_not_notify() {
        let (event_tx, mut event_rx) = mpsc::channel(10);

        publish_analyses(&[analysis(true, None, "critical")], &event_tx).await;
        drop(event_tx);

        let events = drain(&mut event_rx).await;
        assert_eq!(events.len(), 1, "no message means no notification");
        match &events[0] {
            AgUiEvent::TelemetryEvent {
                event_type,
                details,
                ..
            } => {
                assert_eq!(event_type, "health_summary");
                assert_eq!(details["unhealthy"], 1);
                assert_eq!(details["healthy"], 0);
            }
            other => panic!("Expected health_summary, got {other:?}"),
        }
    }

    // The verdict comes from a live local model, so this asserts what holds for
    // any verdict: one summary per batch, preceded by no more notifications than
    // it reports unhealthy commands (an unhealthy verdict may carry no message).
    // Never receiving the summary fails the test rather than passing silently.
    #[tokio::test]
    #[ignore = "requires a local model that produces structured timeout analyses"]
    async fn handler_emits_summary_consistent_with_notifications() {
        let (event_tx, mut event_rx) = mpsc::channel(10);
        let (batch_tx, batch_rx) = mpsc::channel(10);

        let handler = TimeoutHandler::new_offline().await.unwrap();
        let _handle = handler.start(batch_rx, event_tx).await;

        let batch = HealthBatch {
            commands: vec![CommandHealthData {
                id: 1,
                operation: "ReplaceInnerHTML".to_string(),
                selector: "#test".to_string(),
                duration_ms: 35000,
                component: "cef".to_string(),
            }],
            timestamp: chrono::Utc::now(),
        };
        batch_tx.send(batch).await.unwrap();

        let mut notifications = 0;
        let summary = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            loop {
                match event_rx.recv().await.expect("handler closed event channel") {
                    AgUiEvent::SystemNotification { message, .. } => {
                        assert!(!message.is_empty());
                        notifications += 1;
                    }
                    AgUiEvent::TelemetryEvent {
                        event_type,
                        details,
                        ..
                    } if event_type == "health_summary" => break details,
                    other => panic!("Unexpected event {other:?}"),
                }
            }
        })
        .await
        .expect("health_summary never arrived");

        assert_eq!(summary["total"], 1);
        let unhealthy = summary["unhealthy"].as_u64().unwrap();
        assert!(notifications <= unhealthy);
    }
}
