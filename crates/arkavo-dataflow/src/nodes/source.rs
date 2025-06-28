use super::NodeProcessor;
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;

pub struct TimerSource;

#[async_trait]
impl NodeProcessor for TimerSource {
    async fn process(
        &self,
        _input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        // Timer source generates events at intervals
        let interval_ms = params
            .get("interval_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);

        // In a real implementation, this would be triggered by a timer
        // For now, just return a timestamp event
        Ok(Some(json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "source": "timer",
            "interval_ms": interval_ms,
        })))
    }

    fn node_type(&self) -> &'static str {
        "source"
    }
}

pub struct WebhookSource;

#[async_trait]
impl NodeProcessor for WebhookSource {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        // Webhook source would receive data from HTTP requests
        // For now, just pass through the input with metadata
        let endpoint = params
            .get("endpoint")
            .and_then(|v| v.as_str())
            .unwrap_or("/webhook");

        if let Some(data) = input {
            Ok(Some(json!({
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "source": "webhook",
                "endpoint": endpoint,
                "data": data,
            })))
        } else {
            Ok(None)
        }
    }

    fn node_type(&self) -> &'static str {
        "source"
    }
}

pub struct TaskWindowSource;

#[async_trait]
impl NodeProcessor for TaskWindowSource {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        // This source connects to a terminal task window
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("task_id parameter required"))?;

        if let Some(data) = input {
            Ok(Some(json!({
                "timestamp": chrono::Utc::now().to_rfc3339(),
                "source": "task_window",
                "task_id": task_id,
                "data": data,
            })))
        } else {
            Ok(None)
        }
    }

    fn node_type(&self) -> &'static str {
        "source"
    }
}

pub struct FileWatchSource;

#[async_trait]
impl NodeProcessor for FileWatchSource {
    async fn process(
        &self,
        _input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>> {
        // Watch for file changes
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("path parameter required"))?;

        let event_type = params
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or("modified");

        // In a real implementation, this would be triggered by file system events
        Ok(Some(json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "source": "file_watch",
            "path": path,
            "event_type": event_type,
        })))
    }

    fn node_type(&self) -> &'static str {
        "source"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_timer_source() {
        let source = TimerSource;
        let mut params = HashMap::new();
        params.insert("interval_ms".to_string(), json!(5000));

        let result = source.process(None, &params).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output["source"], "timer");
        assert_eq!(output["interval_ms"], 5000);
        assert!(output["timestamp"].is_string());
    }

    #[tokio::test]
    async fn test_webhook_source() {
        let source = WebhookSource;
        let mut params = HashMap::new();
        params.insert("endpoint".to_string(), json!("/api/data"));

        let input = json!({"test": "data"});
        let result = source.process(Some(input.clone()), &params).await.unwrap();
        assert!(result.is_some());

        let output = result.unwrap();
        assert_eq!(output["source"], "webhook");
        assert_eq!(output["endpoint"], "/api/data");
        assert_eq!(output["data"], input);
    }
}
