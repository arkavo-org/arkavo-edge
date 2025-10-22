use crate::server::{Tool, ToolSchema};
use crate::ToolError;
use arkavo_browser::BrowserTool as BrowserToolImpl;
use arkavo_mcp::Tool as McpTool;
use async_trait::async_trait;
use serde_json::Value;

pub struct BrowserTool {
    inner: BrowserToolImpl,
}

impl BrowserTool {
    pub fn new() -> Self {
        Self {
            inner: BrowserToolImpl::new(),
        }
    }
}

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowserTool {
    async fn execute(&self, params: Value) -> crate::Result<Value> {
        // Call the trait method explicitly
        McpTool::execute(&self.inner, params)
            .await
            .map_err(|e| ToolError::Execution(e.to_string()))
    }

    fn schema(&self) -> &ToolSchema {
        // Both traits use the same ToolSchema from arkavo-mcp
        McpTool::schema(&self.inner)
    }
}
