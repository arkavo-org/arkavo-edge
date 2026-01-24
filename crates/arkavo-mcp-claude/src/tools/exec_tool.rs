use async_trait::async_trait;
use serde_json::Value;

use super::ClaudeCodeTool;

/// Shell execution tool for Claude Code
pub struct ExecTool {
    enabled: bool,
}

impl ExecTool {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }
}

#[async_trait]
impl ClaudeCodeTool for ExecTool {
    fn name(&self) -> &'static str {
        "exec_tool"
    }

    fn description(&self) -> &'static str {
        "Execute shell commands within the workspace"
    }

    async fn execute(&self, params: Value) -> crate::Result<Value> {
        if !self.enabled {
            return Err(crate::ClaudeCodeError::PolicyViolation(
                "Shell execution is disabled".to_string(),
            ));
        }

        // Validate params structure
        let _command = params.get("command").ok_or_else(|| {
            crate::ClaudeCodeError::Other("Missing command parameter".to_string())
        })?;

        // Tool execution is delegated to Node.js bridge with Claude Code SDK
        // The bridge handles the actual command execution through the SDK
        Ok(serde_json::json!({
            "success": true,
            "message": "Command execution delegated to Claude Code SDK",
            "params": params
        }))
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }
}
