use async_trait::async_trait;
use serde_json::Value;

// Re-export from arkavo-mcp for consistency
pub use arkavo_mcp::ToolSchema;

/// Core trait for MCP tools
#[async_trait]
pub trait Tool: Send + Sync {
    /// Execute the tool with the given parameters
    async fn execute(&self, params: Value) -> crate::Result<Value>;

    /// Get the tool's schema definition
    fn schema(&self) -> &ToolSchema;
}

// Helper function to create a standard error response
pub fn error_response(message: &str) -> Value {
    serde_json::json!({
        "success": false,
        "error": message
    })
}

// Helper function to create a standard success response
pub fn success_response(data: Value) -> Value {
    serde_json::json!({
        "success": true,
        "data": data
    })
}
