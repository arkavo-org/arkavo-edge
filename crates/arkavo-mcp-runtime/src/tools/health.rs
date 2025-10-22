use anyhow::Result;
use arkavo_mcp_core::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};

/// Health check tool
#[derive(Debug)]
pub struct HealthTool;

impl HealthTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HealthTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for HealthTool {
    async fn execute(&self, _params: Value) -> Result<Value> {
        Ok(json!({
            "status": "healthy",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "version": env!("CARGO_PKG_VERSION"),
        }))
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| ToolSchema {
            name: "health".to_string(),
            description: "Check the health status of the MCP server".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
            }),
        });
        &SCHEMA
    }
}
