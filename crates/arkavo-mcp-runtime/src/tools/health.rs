use anyhow::Result;
use arkavo_mcp_core::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{Value, json};

/// Health check tool
#[derive(Debug)]
pub struct HealthTool;

impl HealthTool {
    pub fn new() -> Self {
        Self
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
        static SCHEMA: once_cell::sync::Lazy<ToolSchema> =
            once_cell::sync::Lazy::new(|| ToolSchema {
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
