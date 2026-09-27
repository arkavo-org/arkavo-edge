use arkavo_mcp_tools::ToolRegistry;
use arkavo_memory::MemoryStorage;
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
    /// MCP-I identity proof and metadata
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none", default)]
    pub meta: Option<Value>,
}

pub struct A2aMcpBridge {
    registry: Arc<RwLock<ToolRegistry>>,
}

impl A2aMcpBridge {
    pub async fn new() -> Result<Self, arkavo_memory::error::MemoryError> {
        let storage = Arc::new(MemoryStorage::new().await?);
        Ok(Self {
            registry: Arc::new(RwLock::new(ToolRegistry::new(storage))),
        })
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
                    meta: None,
                };
            }
        };

        match tool.execute(request.params).await {
            Ok(result) => McpToolResponse {
                success: true,
                result,
                error: None,
                meta: None,
            },
            Err(e) => McpToolResponse {
                success: false,
                result: Value::Null,
                error: Some(e.to_string()),
                meta: None,
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

// Note: Default cannot be implemented for A2aMcpBridge since new() is async.

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use serde_json::json;

    // Each test gets an isolated SQLite path via MemoryStorage::new_test() —
    // sharing the default path causes "database is locked" under parallel test runs.
    async fn test_bridge() -> A2aMcpBridge {
        let storage = Arc::new(
            MemoryStorage::new_test()
                .await
                .expect("Failed to create test storage"),
        );
        A2aMcpBridge {
            registry: Arc::new(RwLock::new(ToolRegistry::new(storage))),
        }
    }

    #[spec("PROTO-005")]
    #[tokio::test]
    async fn test_bridge_creation() {
        let bridge = test_bridge().await;
        let tools = bridge.list_tools().await;
        assert!(!tools.is_empty());
    }

    #[spec("PROTO-005")]
    #[tokio::test]
    async fn test_call_get_agent_time() {
        let bridge = test_bridge().await;
        let request = McpToolRequest {
            tool_name: "get_agent_time".to_string(),
            params: json!({"format": "unix"}),
        };

        let response = bridge.call_tool(request).await;
        assert!(response.success);
        assert!(response.error.is_none());
        assert!(response.result["unix_seconds"].is_number());
    }

    #[spec("PROTO-005")]
    #[tokio::test]
    async fn test_call_nonexistent_tool() {
        let bridge = test_bridge().await;
        let request = McpToolRequest {
            tool_name: "nonexistent_tool".to_string(),
            params: json!({}),
        };

        let response = bridge.call_tool(request).await;
        assert!(!response.success);
        assert!(response.error.is_some());
        assert!(response.error.unwrap().contains("not found"));
    }

    #[spec("PROTO-005")]
    #[tokio::test]
    async fn test_list_tools_includes_time_tools() {
        let bridge = test_bridge().await;
        let tools = bridge.list_tools().await;

        assert!(tools.contains(&"get_agent_time".to_string()));
        assert!(tools.contains(&"sync_agent_time".to_string()));
        assert!(tools.contains(&"get_time_status".to_string()));
    }

    #[spec("PROTO-005")]
    #[tokio::test]
    async fn test_get_tool_schema() {
        let bridge = test_bridge().await;
        let schema = bridge.get_tool_schema("get_agent_time").await;

        assert!(schema.is_some());
        let schema = schema.unwrap();
        assert_eq!(schema["name"], "get_agent_time");
        assert!(schema["description"].is_string());
        assert!(schema["parameters"].is_object());
    }
}
