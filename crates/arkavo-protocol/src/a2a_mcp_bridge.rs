use arkavo_mcp_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolRequest {
    pub tool_name: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResponse {
    pub success: bool,
    pub result: Value,
    pub error: Option<String>,
}

pub struct A2aMcpBridge {
    registry: Arc<RwLock<ToolRegistry>>,
}

impl A2aMcpBridge {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(RwLock::new(ToolRegistry::new())),
        }
    }

    pub async fn call_tool(&self, request: McpToolRequest) -> McpToolResponse {
        let registry = self.registry.read().await;

        let tool = match registry.get(&request.tool_name) {
            Some(t) => t,
            None => {
                return McpToolResponse {
                    success: false,
                    result: Value::Null,
                    error: Some(format!("Tool '{}' not found", request.tool_name)),
                };
            }
        };

        match tool.execute(request.params).await {
            Ok(result) => McpToolResponse {
                success: true,
                result,
                error: None,
            },
            Err(e) => McpToolResponse {
                success: false,
                result: Value::Null,
                error: Some(e.to_string()),
            },
        }
    }

    pub async fn list_tools(&self) -> Vec<String> {
        let registry = self.registry.read().await;
        registry
            .list_tools()
            .into_iter()
            .map(|info| info.name)
            .collect()
    }

    pub async fn get_tool_schema(&self, tool_name: &str) -> Option<Value> {
        let registry = self.registry.read().await;
        registry.get(tool_name).map(|tool| {
            let schema = tool.schema();
            serde_json::json!({
                "name": schema.name,
                "description": schema.description,
                "parameters": schema.parameters
            })
        })
    }
}

impl Default for A2aMcpBridge {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_bridge_creation() {
        let bridge = A2aMcpBridge::new();
        let tools = bridge.list_tools().await;
        assert!(!tools.is_empty());
    }

    #[tokio::test]
    async fn test_call_get_agent_time() {
        let bridge = A2aMcpBridge::new();
        let request = McpToolRequest {
            tool_name: "get_agent_time".to_string(),
            params: json!({"format": "unix"}),
        };

        let response = bridge.call_tool(request).await;
        assert!(response.success);
        assert!(response.error.is_none());
        assert!(response.result["unix_seconds"].is_number());
    }

    #[tokio::test]
    async fn test_call_nonexistent_tool() {
        let bridge = A2aMcpBridge::new();
        let request = McpToolRequest {
            tool_name: "nonexistent_tool".to_string(),
            params: json!({}),
        };

        let response = bridge.call_tool(request).await;
        assert!(!response.success);
        assert!(response.error.is_some());
        assert!(response.error.unwrap().contains("not found"));
    }

    #[tokio::test]
    async fn test_list_tools_includes_time_tools() {
        let bridge = A2aMcpBridge::new();
        let tools = bridge.list_tools().await;

        assert!(tools.contains(&"get_agent_time".to_string()));
        assert!(tools.contains(&"sync_agent_time".to_string()));
        assert!(tools.contains(&"get_time_status".to_string()));
    }

    #[tokio::test]
    async fn test_get_tool_schema() {
        let bridge = A2aMcpBridge::new();
        let schema = bridge.get_tool_schema("get_agent_time").await;

        assert!(schema.is_some());
        let schema = schema.unwrap();
        assert_eq!(schema["name"], "get_agent_time");
        assert!(schema["description"].is_string());
        assert!(schema["parameters"].is_object());
    }
}
