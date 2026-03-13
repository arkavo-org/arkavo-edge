use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, warn};

use arkavo_authorization::{AuthorizationClient, Decision};

use crate::config::ClaudeCodeConfig;
use crate::{ClaudeCodeError, Result};

/// Policy bridge for enforcing security policies on Claude Code SDK operations
pub struct PolicyBridge {
    config: ClaudeCodeConfig,
    auth_client: Option<Arc<AuthorizationClient>>,
}

impl PolicyBridge {
    pub fn new(config: ClaudeCodeConfig, auth_client: Option<Arc<AuthorizationClient>>) -> Self {
        Self {
            config,
            auth_client,
        }
    }

    /// Check if a tool request is permitted by policy
    pub async fn check_tool_permission(&self, tool_request: Value) -> Result<bool> {
        let tool_name = tool_request["tool"]
            .as_str()
            .ok_or_else(|| ClaudeCodeError::PolicyViolation("Missing tool name".to_string()))?;

        debug!("Checking permission for tool: {tool_name}");

        // Check basic tool permissions
        match tool_name {
            "read_file" | "list_directory" => {
                if !self.config.tools.read {
                    warn!("Read operations disabled by policy");
                    return Ok(false);
                }
            }
            "write_file" | "create_file" | "edit_file" | "delete_file" => {
                if !self.config.tools.write {
                    warn!("Write operations disabled by policy");
                    return Ok(false);
                }
            }
            "execute_command" | "shell_exec" => {
                if !self.config.tools.exec {
                    warn!("Execute operations disabled by policy");
                    return Ok(false);
                }
            }
            "web_search" | "web_fetch" => {
                if !self.config.tools.web_search {
                    warn!("Web search operations disabled by policy");
                    return Ok(false);
                }
            }
            _ => {
                debug!("Unknown tool: {tool_name}, checking with authorization service");
            }
        }

        // Check file path permissions if applicable
        if let Some(path) = tool_request["path"].as_str()
            && !self.check_path_permission(path)?
        {
            warn!("Path access denied by policy: {path}");
            return Ok(false);
        }

        // Check with authorization service if available
        if let Some(auth_client) = &self.auth_client {
            let decision = self.check_authorization(auth_client, tool_name).await?;
            if decision != Decision::Permit {
                warn!("Tool {tool_name} denied by authorization service");
                return Ok(false);
            }
        }

        debug!("Tool {tool_name} permitted");
        Ok(true)
    }

    /// Check if a file path is allowed by configuration
    fn check_path_permission(&self, path: &str) -> Result<bool> {
        // Convert to absolute path within workspace
        let abs_path = self.resolve_workspace_path(path)?;

        // Check if path is within workspace root
        if !abs_path.starts_with(&self.config.workspace_root) {
            return Err(ClaudeCodeError::PolicyViolation(format!(
                "Path traversal detected: {path} is outside workspace root"
            )));
        }

        // Get relative path for glob matching
        let rel_path = abs_path
            .strip_prefix(&self.config.workspace_root)
            .map_err(|e| ClaudeCodeError::PolicyViolation(format!("Path error: {e}")))?;

        let rel_path_str = rel_path.to_string_lossy();

        // Check against glob patterns
        Ok(self.config.is_path_allowed(&rel_path_str))
    }

    /// Resolve a path within the workspace, preventing traversal
    fn resolve_workspace_path(&self, path: &str) -> Result<PathBuf> {
        let path = Path::new(path);

        // If absolute, ensure it's within workspace
        if path.is_absolute() {
            if !path.starts_with(&self.config.workspace_root) {
                return Err(ClaudeCodeError::PolicyViolation(
                    "Absolute path outside workspace".to_string(),
                ));
            }
            return Ok(path.to_path_buf());
        }

        // Resolve relative path
        let resolved = self.config.workspace_root.join(path);

        // Try to canonicalize if path exists, otherwise normalize manually
        let canonical = if resolved.exists() {
            resolved.canonicalize().map_err(|e| {
                ClaudeCodeError::PolicyViolation(format!("Path resolution error: {e}"))
            })?
        } else {
            // Normalize the path manually without requiring it to exist
            let mut components = Vec::new();
            for component in resolved.components() {
                match component {
                    std::path::Component::ParentDir => {
                        components.pop();
                    }
                    std::path::Component::Normal(c) => {
                        components.push(c.to_os_string());
                    }
                    std::path::Component::RootDir => {
                        components.clear();
                        components.push(std::ffi::OsString::from("/"));
                    }
                    std::path::Component::CurDir => {}
                    std::path::Component::Prefix(_) => {
                        components.push(component.as_os_str().to_os_string());
                    }
                }
            }

            let mut normalized = PathBuf::new();
            for component in components {
                if component == "/" {
                    normalized = PathBuf::from("/");
                } else {
                    normalized.push(component);
                }
            }
            normalized
        };

        // Verify still within workspace after canonicalization
        if !canonical.starts_with(&self.config.workspace_root) {
            return Err(ClaudeCodeError::PolicyViolation(
                "Path traversal detected after canonicalization".to_string(),
            ));
        }

        Ok(canonical)
    }

    /// Check with authorization service
    async fn check_authorization(
        &self,
        auth_client: &AuthorizationClient,
        tool_name: &str,
    ) -> Result<Decision> {
        let token = std::env::var("CLAUDE_CODE_SESSION_ACCESS_TOKEN")
            .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
            .unwrap_or_default();

        if token.is_empty() {
            warn!("No auth token available for authorization check, denying tool: {tool_name}");
            return Ok(Decision::Deny);
        }

        let decision = auth_client
            .authorize_mcp_tool(&token, tool_name)
            .await
            .map_err(|e| ClaudeCodeError::Other(format!("Authorization check failed: {e}")))?;

        Ok(decision)
    }

    /// Audit a tool execution
    pub fn audit_tool_execution(
        &self,
        tool_name: &str,
        params: &Value,
        success: bool,
        error: Option<&str>,
    ) {
        // Log audit event
        let audit_msg = serde_json::json!({
            "event_type": "claude_code_tool_execution",
            "tool_name": tool_name,
            "params": self.redact_sensitive_params(params),
            "success": success,
            "error": error,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        });

        tracing::info!(audit = %audit_msg, "Tool execution audit");
    }

    /// Redact sensitive information from parameters for logging
    fn redact_sensitive_params(&self, params: &Value) -> Value {
        if !self.config.log_redaction {
            return params.clone();
        }

        let mut redacted = params.clone();

        // Redact file contents
        if let Some(content) = redacted.get_mut("content")
            && content.is_string()
        {
            *content = Value::String("[REDACTED]".to_string());
        }

        // Redact command outputs
        if let Some(output) = redacted.get_mut("output")
            && output.is_string()
        {
            *output = Value::String("[REDACTED]".to_string());
        }

        redacted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_validation() {
        let config = ClaudeCodeConfig {
            workspace_root: PathBuf::from("/workspace"),
            allow_globs: vec!["src/**".to_string()],
            deny_globs: vec!["**/.secrets/**".to_string()],
            ..Default::default()
        };

        let bridge = PolicyBridge::new(config, None);

        // Test allowed path
        assert!(bridge.check_path_permission("src/main.rs").is_ok());

        // Test denied path
        assert!(!bridge.check_path_permission(".secrets/api_key").unwrap());

        // Test path traversal
        assert!(bridge.check_path_permission("../outside").is_err());
    }
}
