use crate::server::Tool;
use arkavo_mcp::ToolSchema;
use arkavo_observability::health_reporter::{HealthRegistry, HealthReport};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct HealthCheckTool {
    schema: ToolSchema,
}

impl HealthCheckTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "get_system_health".to_string(),
                description: "Query health status of all system components. Returns health reports for UI generation, routing, LLM providers, and other registered components.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "component": {
                            "type": "string",
                            "description": "Optional: specific component to check. If not provided, returns all components."
                        }
                    },
                    "required": []
                }),
            },
        }
    }
}

impl Default for HealthCheckTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for HealthCheckTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let registry = HealthRegistry::global();

        let reports: Vec<HealthReport> =
            if let Some(component) = args.get("component").and_then(|v| v.as_str()) {
                if let Some(report) = registry.check_component(component).await {
                    vec![report]
                } else {
                    return Ok(json!({
                        "error": format!("Component '{}' not found", component),
                        "available_components": get_registered_components(registry).await
                    }));
                }
            } else {
                registry.check_all().await
            };

        let overall_status = registry.get_overall_status().await;

        Ok(json!({
            "overall_status": format!("{:?}", overall_status).to_lowercase(),
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "components": reports,
            "summary": {
                "total": reports.len(),
                "healthy": reports.iter().filter(|r| matches!(r.status, arkavo_observability::health_reporter::HealthStatus::Healthy)).count(),
                "degraded": reports.iter().filter(|r| matches!(r.status, arkavo_observability::health_reporter::HealthStatus::Degraded)).count(),
                "unhealthy": reports.iter().filter(|r| matches!(r.status, arkavo_observability::health_reporter::HealthStatus::Unhealthy)).count(),
            }
        }))
    }
}

async fn get_registered_components(registry: &HealthRegistry) -> Vec<String> {
    registry
        .check_all()
        .await
        .into_iter()
        .map(|r| r.component)
        .collect()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_check_schema() {
        let tool = HealthCheckTool::new();
        let schema = tool.schema();

        assert_eq!(schema.name, "get_system_health");
        assert!(schema.description.contains("health status"));
    }

    #[tokio::test]
    async fn test_health_check_execution() {
        let tool = HealthCheckTool::new();
        let result = tool.execute(json!({})).await.unwrap();

        assert!(result.get("overall_status").is_some());
        assert!(result.get("components").is_some());
        assert!(result.get("summary").is_some());
    }
}
