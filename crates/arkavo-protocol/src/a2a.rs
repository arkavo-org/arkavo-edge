use crate::a2a_mcp_bridge::{A2aMcpBridge, McpToolRequest, McpToolResponse};
use serde_json::Value;

pub struct A2aClient {
    mcp_bridge: Option<A2aMcpBridge>,
}

impl Default for A2aClient {
    fn default() -> Self {
        Self::new()
    }
}

impl A2aClient {
    pub fn new() -> Self {
        Self { mcp_bridge: None }
    }

    pub fn with_mcp_bridge(mcp_bridge: A2aMcpBridge) -> Self {
        Self {
            mcp_bridge: Some(mcp_bridge),
        }
    }

    pub fn send(&self, _message: &str) -> Result<String, Box<dyn std::error::Error>> {
        Ok("A2A response".to_string())
    }

    pub async fn call_mcp_tool(
        &self,
        tool_name: &str,
        params: Value,
    ) -> Result<McpToolResponse, Box<dyn std::error::Error>> {
        let bridge = self
            .mcp_bridge
            .as_ref()
            .ok_or("MCP bridge not initialized")?;

        let request = McpToolRequest {
            tool_name: tool_name.to_string(),
            params,
        };

        Ok(bridge.call_tool(request).await)
    }
}
