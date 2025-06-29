use anyhow::Result;
use arkavo_mcp_core::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{Value, json};

/// Simple echo tool for testing
#[derive(Debug)]
pub struct EchoTool;

impl EchoTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for EchoTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        Ok(json!({
            "echo": params,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }))
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: once_cell::sync::Lazy<ToolSchema> =
            once_cell::sync::Lazy::new(|| ToolSchema {
                name: "echo".to_string(),
                description: "Echo back the input parameters".to_string(),
                parameters: json!({
                    "type": "object",
                    "additionalProperties": true,
                }),
            });
        &SCHEMA
    }
}
