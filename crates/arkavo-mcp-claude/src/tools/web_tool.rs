use async_trait::async_trait;
use serde_json::Value;

use super::ClaudeCodeTool;

/// Web search tool for Claude Code
pub struct WebTool {
    enabled: bool,
}

impl WebTool {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }
}

#[async_trait]
impl ClaudeCodeTool for WebTool {
    fn name(&self) -> &'static str {
        "web_tool"
    }

    fn description(&self) -> &'static str {
        "Search and fetch information from the web"
    }

    async fn execute(&self, params: Value) -> crate::Result<Value> {
        if !self.enabled {
            return Err(crate::ClaudeCodeError::PolicyViolation(
                "Web search is disabled".to_string(),
            ));
        }

        // Validate params structure
        let _query = params
            .get("query")
            .ok_or_else(|| crate::ClaudeCodeError::Other("Missing query parameter".to_string()))?;

        // Tool execution is delegated to Node.js bridge with Claude Code SDK
        // The bridge handles the actual web search through the SDK
        Ok(serde_json::json!({
            "success": true,
            "message": "Web search delegated to Claude Code SDK",
            "params": params
        }))
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }
}
