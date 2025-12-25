use arkavo_mcp_tools::registry::ToolRegistry;
use arkavo_memory::MemoryStorage;
use serde_json::Value;
use thiserror::Error;

use crate::tool_parser::ParsedToolCall;

#[derive(Debug, Error)]
pub enum ToolExecutionError {
    #[error("Tool not found: {0}")]
    ToolNotFound(String),
    #[error("Tool execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),
    #[error("Authorization denied: {0}")]
    AuthorizationDenied(String),
}

/// Result of tool execution
#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    pub tool_name: String,
    pub call_id: Option<String>,
    pub result: Value,
    pub success: bool,
    pub error: Option<String>,
}

use std::sync::Arc;

/// Tool executor that runs MCP tools
pub struct ToolExecutor {
    registry: Arc<ToolRegistry>,
}

impl ToolExecutor {
    pub fn new(storage: Arc<MemoryStorage>) -> Self {
        Self {
            registry: Arc::new(ToolRegistry::new(storage)),
        }
    }

    pub fn with_registry(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }

    /// Execute a single parsed tool call
    pub async fn execute(
        &self,
        tool_call: &ParsedToolCall,
    ) -> Result<ToolExecutionResult, ToolExecutionError> {
        let tool = self
            .registry
            .get(&tool_call.tool_name)
            .ok_or_else(|| ToolExecutionError::ToolNotFound(tool_call.tool_name.clone()))?;

        match tool.execute(tool_call.arguments.clone()).await {
            Ok(result) => Ok(ToolExecutionResult {
                tool_name: tool_call.tool_name.clone(),
                call_id: tool_call.call_id.clone(),
                result,
                success: true,
                error: None,
            }),
            Err(e) => {
                let error_msg = e.to_string();
                Ok(ToolExecutionResult {
                    tool_name: tool_call.tool_name.clone(),
                    call_id: tool_call.call_id.clone(),
                    result: serde_json::json!({"error": error_msg}),
                    success: false,
                    error: Some(error_msg),
                })
            }
        }
    }

    /// Execute multiple tool calls in sequence
    pub async fn execute_batch(&self, tool_calls: &[ParsedToolCall]) -> Vec<ToolExecutionResult> {
        let mut results = Vec::new();
        for call in tool_calls {
            match self.execute(call).await {
                Ok(result) => results.push(result),
                Err(e) => {
                    results.push(ToolExecutionResult {
                        tool_name: call.tool_name.clone(),
                        call_id: call.call_id.clone(),
                        result: serde_json::json!({"error": e.to_string()}),
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }
        results
    }

    /// Check if a tool exists in the registry
    pub fn has_tool(&self, tool_name: &str) -> bool {
        self.registry.get(tool_name).is_some()
    }

    /// Get list of available tool names
    pub fn available_tools(&self) -> Vec<String> {
        self.registry
            .list_tools()
            .iter()
            .map(|t| t.name.clone())
            .collect()
    }

    /// Get the tool registry for converting to provider formats
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }
}

// Note: Default cannot be implemented for ToolExecutor since it requires async storage initialization.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn create_test_executor() -> ToolExecutor {
        let storage = Arc::new(MemoryStorage::new_test().await.expect("Storage init"));
        ToolExecutor::new(storage)
    }

    #[tokio::test]
    async fn test_executor_creation() {
        let executor = create_test_executor().await;
        let tools = executor.available_tools();
        assert!(!tools.is_empty(), "Should have MCP tools registered");
    }

    #[tokio::test]
    async fn test_has_tool() {
        let executor = create_test_executor().await;
        let tools = executor.available_tools();
        if let Some(first_tool) = tools.first() {
            assert!(executor.has_tool(first_tool));
        }
        assert!(!executor.has_tool("nonexistent_tool"));
    }

    #[tokio::test]
    async fn test_execute_nonexistent_tool() {
        let executor = create_test_executor().await;
        let call = ParsedToolCall {
            tool_name: "nonexistent_tool".to_string(),
            arguments: json!({}),
            call_id: Some("test_123".to_string()),
        };

        let result = executor.execute(&call).await;
        assert!(result.is_err());
        match result {
            Err(ToolExecutionError::ToolNotFound(name)) => {
                assert_eq!(name, "nonexistent_tool");
            }
            _ => panic!("Expected ToolNotFound error"),
        }
    }

    #[tokio::test]
    async fn test_execute_batch_empty() {
        let executor = create_test_executor().await;
        let results = executor.execute_batch(&[]).await;
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_execute_batch_with_errors() {
        let executor = create_test_executor().await;
        let calls = vec![
            ParsedToolCall {
                tool_name: "nonexistent_1".to_string(),
                arguments: json!({}),
                call_id: Some("call_1".to_string()),
            },
            ParsedToolCall {
                tool_name: "nonexistent_2".to_string(),
                arguments: json!({}),
                call_id: Some("call_2".to_string()),
            },
        ];

        let results = executor.execute_batch(&calls).await;
        assert_eq!(results.len(), 2);
        assert!(!results[0].success);
        assert!(!results[1].success);
        assert!(results[0].error.is_some());
        assert!(results[1].error.is_some());
    }

    #[tokio::test]
    async fn test_registry_access() {
        let executor = create_test_executor().await;
        let registry = executor.registry();
        let tools = registry.list_tools();
        assert!(!tools.is_empty());
    }
}
