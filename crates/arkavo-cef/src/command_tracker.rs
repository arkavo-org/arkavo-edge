use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct CommandRecord {
    pub id: u32,
    pub op: u8,
    pub selector: String,
    pub sent_at: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHealthSnapshot {
    pub commands: Vec<CommandHealthReport>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandHealthReport {
    pub id: u32,
    pub op_name: String,
    pub selector: String,
    pub duration_ms: u64,
}

pub struct CommandTracker {
    active_commands: Arc<RwLock<HashMap<u32, CommandRecord>>>,
}

impl CommandTracker {
    pub fn new() -> Self {
        Self {
            active_commands: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn track_command(&self, id: u32, op: u8, selector: String) {
        let record = CommandRecord {
            id,
            op,
            selector,
            sent_at: Instant::now(),
        };
        self.active_commands.write().await.insert(id, record);
    }

    pub async fn complete_command(&self, id: u32) -> Option<Duration> {
        self.active_commands
            .write()
            .await
            .remove(&id)
            .map(|record| record.sent_at.elapsed())
    }

    pub async fn get_health_snapshot(&self) -> CommandHealthSnapshot {
        let reports: Vec<CommandHealthReport> = self
            .active_commands
            .read()
            .await
            .values()
            .map(|record| CommandHealthReport {
                id: record.id,
                op_name: Self::op_to_name(record.op),
                selector: record.selector.clone(),
                duration_ms: record.sent_at.elapsed().as_millis() as u64,
            })
            .collect();

        CommandHealthSnapshot {
            commands: reports,
            timestamp: chrono::Utc::now(),
        }
    }

    fn op_to_name(op: u8) -> String {
        match op {
            0 => "ReplaceInnerHTML",
            1 => "SetAttribute",
            2 => "SetStyle",
            3 => "RemoveNode",
            4 => "AppendNode",
            5 => "QuerySelector",
            6 => "AddEventListener",
            7 => "SetTextContent",
            _ => "Unknown",
        }
        .to_string()
    }
}

impl Default for CommandTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_command_tracking() {
        let tracker = CommandTracker::new();
        tracker.track_command(1, 0, "#test".to_string()).await;

        tokio::time::sleep(Duration::from_millis(100)).await;
        let snapshot = tracker.get_health_snapshot().await;

        assert_eq!(snapshot.commands.len(), 1);
        assert!(snapshot.commands[0].duration_ms >= 100);
        assert_eq!(snapshot.commands[0].op_name, "ReplaceInnerHTML");
    }

    #[tokio::test]
    async fn test_command_completion() {
        let tracker = CommandTracker::new();
        tracker.track_command(1, 0, "#test".to_string()).await;

        tokio::time::sleep(Duration::from_millis(50)).await;
        let duration = tracker.complete_command(1).await;

        assert!(duration.is_some());
        assert!(duration.unwrap().as_millis() >= 50);

        let snapshot = tracker.get_health_snapshot().await;
        assert_eq!(snapshot.commands.len(), 0);
    }

    #[tokio::test]
    async fn test_multiple_commands() {
        let tracker = CommandTracker::new();
        tracker.track_command(1, 0, "#test1".to_string()).await;
        tracker.track_command(2, 1, "#test2".to_string()).await;
        tracker.track_command(3, 2, "#test3".to_string()).await;

        let snapshot = tracker.get_health_snapshot().await;
        assert_eq!(snapshot.commands.len(), 3);

        tracker.complete_command(2).await;
        let snapshot = tracker.get_health_snapshot().await;
        assert_eq!(snapshot.commands.len(), 2);
    }
}
