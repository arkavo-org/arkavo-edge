pub mod exec_tool;
pub mod file_tools;
pub mod web_tool;

use async_trait::async_trait;
use serde_json::Value;

/// Trait for Claude Code tools
#[async_trait]
pub trait ClaudeCodeTool: Send + Sync {
    /// Get the tool name
    fn name(&self) -> &str;

    /// Get the tool description
    fn description(&self) -> &str;

    /// Execute the tool with given parameters
    async fn execute(&self, params: Value) -> crate::Result<Value>;

    /// Check if the tool is enabled
    fn is_enabled(&self) -> bool;
}
