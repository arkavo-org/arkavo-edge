use crate::mcp_registry::McpRegistry;
use async_trait::async_trait;
use std::sync::Arc;

/// Bridge tool that wraps an MCP tool from McpRegistry
///
/// This implements the Tool trait for ToolRegistry, but delegates
/// actual execution to the McpRegistry (the source of truth for
/// active MCP connections).
pub struct McpBridgeTool {
    registry: Arc<McpRegistry>,
    schema: arkavo_mcp_tools::ToolSchema,
}

impl McpBridgeTool {
    pub fn new(registry: Arc<McpRegistry>, tool: crate::mcp_registry::Tool) -> Self {
        Self {
            registry,
            schema: arkavo_mcp_tools::ToolSchema {
                name: tool.name,
                aliases: None,
                description: tool.description,
                parameters: tool
                    .input_schema
                    .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
            },
        }
    }
}

#[async_trait]
impl arkavo_mcp_tools::server::Tool for McpBridgeTool {
    fn schema(&self) -> &arkavo_mcp_tools::ToolSchema {
        &self.schema
    }

    async fn execute(
        &self,
        params: serde_json::Value,
    ) -> arkavo_mcp_tools::Result<serde_json::Value> {
        self.registry
            .call_tool(&self.schema.name, params, "hrm-conductor")
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))
    }
}
