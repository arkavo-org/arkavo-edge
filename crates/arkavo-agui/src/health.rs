use arkavo_observability::health_reporter::{HealthReport, HealthReporter, HealthStatus};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Default)]
struct WebSocketMetrics {
    active_connections: usize,
    total_connections: u64,
    failed_connections: u64,
    message_queue_depth: usize,
    last_connection_time: Option<DateTime<Utc>>,
    last_error: Option<(DateTime<Utc>, String)>,
}

#[derive(Debug, Clone, Default)]
struct McpToolsStatus {
    tools_count: usize,
    browser_available: bool,
    github_available: bool,
    last_tool_usage: Option<DateTime<Utc>>,
    tool_failures: u32,
}

#[derive(Debug, Clone)]
struct MdnsStatus {
    agents_discovered: usize,
    last_discovery: Option<DateTime<Utc>>,
    discovery_enabled: bool,
}

impl Default for MdnsStatus {
    fn default() -> Self {
        Self {
            agents_discovered: 0,
            last_discovery: None,
            discovery_enabled: true,
        }
    }
}

pub struct AguiHealthReporter {
    ws_metrics: Arc<RwLock<WebSocketMetrics>>,
    mcp_status: Arc<RwLock<McpToolsStatus>>,
    mdns_status: Arc<RwLock<MdnsStatus>>,
}

impl AguiHealthReporter {
    pub fn new() -> Self {
        Self {
            ws_metrics: Arc::new(RwLock::new(WebSocketMetrics::default())),
            mcp_status: Arc::new(RwLock::new(McpToolsStatus::default())),
            mdns_status: Arc::new(RwLock::new(MdnsStatus::default())),
        }
    }

    pub async fn record_connection(&self, success: bool) {
        let mut metrics = self.ws_metrics.write().await;
        metrics.total_connections += 1;

        if success {
            metrics.active_connections += 1;
            metrics.last_connection_time = Some(Utc::now());
        } else {
            metrics.failed_connections += 1;
        }
    }

    pub async fn record_disconnection(&self) {
        let mut metrics = self.ws_metrics.write().await;
        metrics.active_connections = metrics.active_connections.saturating_sub(1);
    }

    pub async fn record_connection_error(&self, error: String) {
        let mut metrics = self.ws_metrics.write().await;
        metrics.last_error = Some((Utc::now(), error));
    }

    pub async fn update_message_queue_depth(&self, depth: usize) {
        let mut metrics = self.ws_metrics.write().await;
        metrics.message_queue_depth = depth;
    }

    pub async fn update_mcp_tools(&self, count: usize, browser: bool, github: bool) {
        let mut status = self.mcp_status.write().await;
        status.tools_count = count;
        status.browser_available = browser;
        status.github_available = github;
    }

    pub async fn record_tool_usage(&self, success: bool) {
        let mut status = self.mcp_status.write().await;
        if success {
            status.last_tool_usage = Some(Utc::now());
        } else {
            status.tool_failures += 1;
        }
    }

    pub async fn update_mdns_discovery(&self, agents_count: usize) {
        let mut status = self.mdns_status.write().await;
        status.agents_discovered = agents_count;
        status.last_discovery = Some(Utc::now());
    }

    pub async fn get_active_connections(&self) -> usize {
        self.ws_metrics.read().await.active_connections
    }
}

impl Default for AguiHealthReporter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HealthReporter for AguiHealthReporter {
    async fn check_health(&self) -> HealthReport {
        let ws_metrics = self.ws_metrics.read().await;
        let mcp_status = self.mcp_status.read().await;
        let mdns_status = self.mdns_status.read().await;

        // Determine health status
        let status = if let Some((error_time, _)) = &ws_metrics.last_error {
            if Utc::now().signed_duration_since(*error_time).num_seconds() < 60 {
                HealthStatus::Unhealthy
            } else {
                HealthStatus::Degraded
            }
        } else if ws_metrics.message_queue_depth > 100
            || (ws_metrics.active_connections == 0 && ws_metrics.total_connections > 0)
            || mcp_status.tool_failures > 5
        {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };

        let message = match status {
            HealthStatus::Healthy => {
                format!(
                    "AG-UI Gateway healthy. {} active connections, {} MCP tools available",
                    ws_metrics.active_connections, mcp_status.tools_count
                )
            }
            HealthStatus::Degraded => {
                if ws_metrics.message_queue_depth > 100 {
                    format!(
                        "High message queue depth: {} pending messages",
                        ws_metrics.message_queue_depth
                    )
                } else if ws_metrics.active_connections == 0 {
                    "No active WebSocket connections".to_string()
                } else if mcp_status.tool_failures > 5 {
                    format!("MCP tool failures: {}", mcp_status.tool_failures)
                } else {
                    "Gateway experiencing recent errors".to_string()
                }
            }
            HealthStatus::Unhealthy => {
                if let Some((_, error)) = &ws_metrics.last_error {
                    format!("WebSocket error: {}", error)
                } else {
                    "Gateway is experiencing critical errors".to_string()
                }
            }
            HealthStatus::Unknown => "AG-UI Gateway health unknown".to_string(),
        };

        let mut details = HashMap::new();
        details.insert(
            "active_connections".to_string(),
            serde_json::json!(ws_metrics.active_connections),
        );
        details.insert(
            "total_connections".to_string(),
            serde_json::json!(ws_metrics.total_connections),
        );
        details.insert(
            "failed_connections".to_string(),
            serde_json::json!(ws_metrics.failed_connections),
        );
        details.insert(
            "message_queue_depth".to_string(),
            serde_json::json!(ws_metrics.message_queue_depth),
        );
        details.insert(
            "connection_success_rate".to_string(),
            serde_json::json!(if ws_metrics.total_connections > 0 {
                ((ws_metrics.total_connections - ws_metrics.failed_connections) as f64
                    / ws_metrics.total_connections as f64)
                    * 100.0
            } else {
                100.0
            }),
        );

        details.insert(
            "mcp_tools_count".to_string(),
            serde_json::json!(mcp_status.tools_count),
        );
        details.insert(
            "browser_available".to_string(),
            serde_json::json!(mcp_status.browser_available),
        );
        details.insert(
            "github_available".to_string(),
            serde_json::json!(mcp_status.github_available),
        );
        details.insert(
            "tool_failures".to_string(),
            serde_json::json!(mcp_status.tool_failures),
        );

        details.insert(
            "mdns_agents_discovered".to_string(),
            serde_json::json!(mdns_status.agents_discovered),
        );
        details.insert(
            "mdns_discovery_enabled".to_string(),
            serde_json::json!(mdns_status.discovery_enabled),
        );

        HealthReport {
            component: "agui-gateway".to_string(),
            status,
            message,
            details: Some(details),
            timestamp: Utc::now(),
        }
    }

    fn component_name(&self) -> &str {
        "agui-gateway"
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_healthy_gateway() {
        let reporter = AguiHealthReporter::new();

        // Record healthy activity
        reporter.record_connection(true).await;
        reporter.update_mcp_tools(10, true, true).await;
        reporter.update_mdns_discovery(3).await;

        let report = reporter.check_health().await;
        assert_eq!(report.status, HealthStatus::Healthy);
        assert_eq!(report.component, "agui-gateway");
    }

    #[tokio::test]
    async fn test_degraded_high_queue() {
        let reporter = AguiHealthReporter::new();

        // Simulate high queue depth
        reporter.update_message_queue_depth(150).await;

        let report = reporter.check_health().await;
        assert_eq!(report.status, HealthStatus::Degraded);
        assert!(report.message.contains("queue depth"));
    }

    #[tokio::test]
    async fn test_unhealthy_websocket_error() {
        let reporter = AguiHealthReporter::new();

        // Record recent error
        reporter
            .record_connection_error("Connection refused".to_string())
            .await;

        let report = reporter.check_health().await;
        assert_eq!(report.status, HealthStatus::Unhealthy);
        assert!(report.message.contains("WebSocket error"));
    }
}
