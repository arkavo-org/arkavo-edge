use crate::types::AgUiEvent;
use anyhow::Result;
use arkavo_llm::{Message, Role};
use arkavo_mcp_tools::registry::ToolRegistry;
use arkavo_router::Router;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HealthAnalysis {
    status: String,
    notify_user: bool,
    user_message: Option<String>,
    severity: Option<String>,
    auto_actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_dismiss_seconds: Option<u32>,
}

pub struct HealthMonitor {
    tool_registry: Arc<ToolRegistry>,
    router: Arc<Router>,
    check_interval: Duration,
}

impl HealthMonitor {
    pub async fn new(tool_registry: Arc<ToolRegistry>) -> Result<Self> {
        // Use router for intelligent model selection
        let router = Arc::new(Router::new().await?);

        Ok(Self {
            tool_registry,
            router,
            check_interval: Duration::from_secs(30),
        })
    }

    pub fn with_interval(mut self, interval_secs: u64) -> Self {
        self.check_interval = Duration::from_secs(interval_secs);
        self
    }

    pub async fn start(
        self,
        event_tx: mpsc::Sender<AgUiEvent>,
    ) -> Result<tokio::task::JoinHandle<()>> {
        let handle = tokio::spawn(async move {
            let mut check_interval = interval(self.check_interval);

            loop {
                check_interval.tick().await;

                if let Err(e) = self.run_health_check(&event_tx).await {
                    eprintln!("Health check error: {e}");
                }
            }
        });

        Ok(handle)
    }

    async fn run_health_check(&self, event_tx: &mpsc::Sender<AgUiEvent>) -> Result<()> {
        // 1. Gather health reports via MCP tool
        let health_tool = self
            .tool_registry
            .get("get_system_health")
            .ok_or_else(|| anyhow::anyhow!("Health check tool 'get_system_health' not found"))?;

        let health_data = health_tool.execute(serde_json::json!({})).await?;

        // 2. Analyze with local LLM
        let analysis = self.analyze_health_with_llm(&health_data).await?;

        // 3. Execute auto-fixes silently
        for action in &analysis.auto_actions {
            if let Err(e) = self.execute_auto_action(action).await {
                eprintln!("Auto-action failed: {action} - {e}");
            }
        }

        // 4. Only send UI update if LLM says user should know
        if analysis.notify_user {
            self.send_health_alert(&analysis, event_tx).await?;
        }

        Ok(())
    }

    async fn analyze_health_with_llm(
        &self,
        health_data: &serde_json::Value,
    ) -> Result<HealthAnalysis> {
        // Try LLM analysis with local model first
        match self.llm_based_analysis(health_data).await {
            Ok(analysis) => Ok(analysis),
            Err(e) => {
                eprintln!("LLM analysis failed: {e}, falling back to rule-based");
                Ok(self.rule_based_analysis(health_data))
            }
        }
    }

    async fn llm_based_analysis(&self, health_data: &serde_json::Value) -> Result<HealthAnalysis> {
        let prompt = self.build_analysis_prompt(health_data);

        // Use router to intelligently select the best model for health analysis
        // The task description hints that this needs structured output generation
        let task_description = "Analyze system health data and generate structured JSON response with actionable insights";

        let _decision = self.router.route(task_description).await?;

        // Get the appropriate provider from router
        let provider = self.router.get_local_provider();

        let messages = vec![Message {
            role: Role::User,
            content: prompt,
            images: None,
        }];

        let response = provider.complete(messages).await?;
        self.parse_analysis(&response)
    }

    fn build_analysis_prompt(&self, health_data: &serde_json::Value) -> String {
        // Extract key health indicators for a simpler prompt
        let components = health_data["components"].as_array();
        let summary = if let Some(comps) = components {
            comps
                .iter()
                .map(|c| {
                    format!(
                        "{}: {} - {}",
                        c["component"].as_str().unwrap_or("unknown"),
                        c["status"].as_str().unwrap_or("unknown"),
                        c["message"].as_str().unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            "No component data".to_string()
        };

        format!(
            r#"Analyze system health and return JSON:

STATUS:
{}

Return this JSON structure:
{{
"status":"healthy",
"notify_user":false,
"user_message":null,
"severity":"info",
"auto_actions":[],
"auto_dismiss_seconds":null
}}

Rules:
- status: "healthy"|"degraded"|"unhealthy"
- notify_user: true only if user needs to know (API errors, config issues, etc.)
- user_message: natural, helpful message explaining impact to user (e.g., "Using local model due to API issues. Generation may be slower.")
- severity: "info"|"warning"|"critical"
- Be concise and user-friendly. Focus on what this means for the user, not technical details.

JSON:"#,
            summary
        )
    }

    fn parse_analysis(&self, response: &str) -> Result<HealthAnalysis> {
        let cleaned = response
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\r' || *c == '\t')
            .collect::<String>();

        let json_str = if let Some(json_start) = cleaned.find("```json") {
            let after_fence = &cleaned[json_start + 7..];
            if let Some(fence_end) = after_fence.find("```") {
                after_fence[..fence_end].trim()
            } else {
                after_fence.trim()
            }
        } else if let Some(start) = cleaned.find('{') {
            let after_start = &cleaned[start..];
            let mut depth = 0;
            let mut end_pos = 0;

            for (i, ch) in after_start.chars().enumerate() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end_pos = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }

            if end_pos > 0 {
                &after_start[..end_pos]
            } else {
                cleaned.trim()
            }
        } else {
            cleaned.trim()
        };

        serde_json::from_str(json_str)
            .or_else(|e| Err(anyhow::anyhow!("Failed to parse LLM health analysis: {e}")))
    }

    fn rule_based_analysis(&self, health_data: &serde_json::Value) -> HealthAnalysis {
        // Simple fallback - just check if any component is unhealthy
        let components = health_data["components"].as_array();

        if let Some(comps) = components {
            for component in comps {
                let status = component["status"].as_str().unwrap_or("unknown");
                let component_name = component["component"].as_str().unwrap_or("system");
                let message = component["message"].as_str().unwrap_or("");

                if status == "unhealthy" {
                    // Generate contextual message based on component and error
                    let user_message = if message.contains("API") || message.contains("Gemini") {
                        format!("Using local model due to API issues. Generation may be slower.")
                    } else if message.contains("auth") || message.contains("key") {
                        format!("API key issue detected. Please check your configuration.")
                    } else {
                        format!(
                            "{} is experiencing issues. System may run slower.",
                            component_name
                        )
                    };

                    return HealthAnalysis {
                        status: "unhealthy".to_string(),
                        notify_user: true,
                        user_message: Some(user_message),
                        severity: Some("warning".to_string()),
                        auto_actions: vec![],
                        auto_dismiss_seconds: Some(15),
                    };
                }

                if status == "degraded" {
                    // Degraded: silent, let auto-fixes handle it
                    return HealthAnalysis {
                        status: "degraded".to_string(),
                        notify_user: false,
                        user_message: None,
                        severity: Some("info".to_string()),
                        auto_actions: vec![],
                        auto_dismiss_seconds: None,
                    };
                }
            }
        }

        // Default: healthy
        HealthAnalysis {
            status: "healthy".to_string(),
            notify_user: false,
            user_message: None,
            severity: None,
            auto_actions: vec![],
            auto_dismiss_seconds: None,
        }
    }

    async fn execute_auto_action(&self, action: &str) -> Result<()> {
        match action {
            "retry_api_connection" => {
                // Trigger reconnection logic
                println!("Auto-action: Retrying API connection");
            }
            "switch_to_local_model" => {
                // Signal router to prefer local models temporarily
                println!("Auto-action: Switching to local model");
            }
            "reconnect_websocket" => {
                // Trigger WebSocket reconnection
                println!("Auto-action: Reconnecting WebSocket");
            }
            "clear_queue" => {
                // Clear stale items from queues
                println!("Auto-action: Clearing stale queue items");
            }
            "throttle_requests" => {
                // Apply rate limiting
                println!("Auto-action: Throttling requests");
            }
            _ => {
                println!("Auto-action not implemented: {action}");
            }
        }
        Ok(())
    }

    async fn send_health_alert(
        &self,
        analysis: &HealthAnalysis,
        event_tx: &mpsc::Sender<AgUiEvent>,
    ) -> Result<()> {
        use crate::types::NotificationSeverity;

        let message = analysis
            .user_message
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No user message in analysis"))?;

        let severity_str = analysis.severity.as_deref().unwrap_or("info");
        let severity = match severity_str {
            "critical" => NotificationSeverity::Error,
            "warning" => NotificationSeverity::Warning,
            _ => NotificationSeverity::Info,
        };

        // Send a simple, contextual notification
        let event = AgUiEvent::SystemNotification {
            message: message.clone(),
            severity,
        };

        event_tx
            .send(event)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send health alert: {e}"))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rule_based_analysis_unhealthy() {
        let monitor = HealthMonitor::new(Arc::new(ToolRegistry::new()))
            .await
            .unwrap();

        let health_data = serde_json::json!({
            "components": [
                {
                    "component": "ui-generator",
                    "status": "unhealthy",
                    "message": "Gemini API error: rate limit exceeded"
                }
            ]
        });

        let analysis = monitor.rule_based_analysis(&health_data);
        assert_eq!(analysis.status, "unhealthy");
        assert!(analysis.notify_user);
        assert_eq!(analysis.severity, Some("warning".to_string()));
        let message = analysis.user_message.unwrap();
        assert!(message.contains("local model") || message.contains("API"));
    }

    #[tokio::test]
    async fn test_rule_based_analysis_degraded() {
        let monitor = HealthMonitor::new(Arc::new(ToolRegistry::new()))
            .await
            .unwrap();

        let health_data = serde_json::json!({
            "components": [
                {
                    "component": "router",
                    "status": "degraded",
                    "message": "High latency detected"
                }
            ]
        });

        let analysis = monitor.rule_based_analysis(&health_data);
        assert_eq!(analysis.status, "degraded");
        assert!(!analysis.notify_user); // Silent auto-fix
    }

    #[tokio::test]
    async fn test_rule_based_analysis_healthy() {
        let monitor = HealthMonitor::new(Arc::new(ToolRegistry::new()))
            .await
            .unwrap();

        let health_data = serde_json::json!({
            "components": [
                {
                    "component": "ui-generator",
                    "status": "healthy",
                    "message": "All systems operational"
                }
            ]
        });

        let analysis = monitor.rule_based_analysis(&health_data);
        assert_eq!(analysis.status, "healthy");
        assert!(!analysis.notify_user);
    }
}
