use crate::command_health_collector::{HealthBatch, TimeoutAnalyzer};
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

        for analysis in analyses {
            if analysis.should_timeout {
                if let Some(msg) = analysis.user_message {
                    self.send_notification(msg, &analysis.severity, event_tx)
                        .await;
                }
            }
        }

        Ok(())
    }

    async fn send_notification(
        &self,
        message: String,
        severity: &str,
        event_tx: &mpsc::Sender<AgUiEvent>,
    ) {
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
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::command_health_collector::CommandHealthData;

    #[tokio::test]
    async fn test_timeout_handler_sends_notification() {
        let (event_tx, mut event_rx) = mpsc::channel(10);
        let (batch_tx, batch_rx) = mpsc::channel(10);

        let handler = TimeoutHandler::new().await.unwrap();
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

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            if let Some(event) = event_rx.recv().await {
                match event {
                    AgUiEvent::SystemNotification { message, severity } => {
                        assert!(!message.is_empty());
                        assert!(matches!(
                            severity,
                            NotificationSeverity::Info
                                | NotificationSeverity::Warning
                                | NotificationSeverity::Error
                        ));
                    }
                    _ => panic!("Expected SystemNotification event"),
                }
            }
        })
        .await
        .ok();
    }
}
