use async_trait::async_trait;
use serde_json::Value;

use super::ClaudeCodeTool;

/// File operations tool for Claude Code
pub struct FileTools {
    enabled: bool,
}

impl FileTools {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }
}

#[async_trait]
impl ClaudeCodeTool for FileTools {
    fn name(&self) -> &str {
        "file_tools"
    }

    fn description(&self) -> &str {
        "File read/write operations within the workspace"
    }

    async fn execute(&self, params: Value) -> crate::Result<Value> {
        if !self.enabled {
            return Err(crate::ClaudeCodeError::PolicyViolation(
                "File operations are disabled".to_string(),
            ));
        }

        // Validate params structure
        let _action = params
            .get("action")
            .ok_or_else(|| crate::ClaudeCodeError::Other("Missing action parameter".to_string()))?;

        // Tool execution is delegated to Node.js bridge with Claude Code SDK
        // The bridge handles the actual file operations through the SDK
        Ok(serde_json::json!({
            "success": true,
            "message": "File operation delegated to Claude Code SDK",
            "params": params
        }))
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }
}
