//! Data security MCP tool for managing local OpenTDF stack.
//!
//! Enables data security mode by starting/stopping a local OpenTDF platform
//! using container orchestration.

use crate::server::Tool;
use arkavo_mcp::ToolSchema;
use arkavo_opentdf_local::{OpenTdfStack, StackStatus};
use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::{info, warn};

/// MCP tool for enabling/disabling data security with local OpenTDF.
pub struct DataSecurityTool {
    schema: ToolSchema,
}

impl DataSecurityTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "enable_data_security".to_string(),
                aliases: Some(vec![
                    "data_security".to_string(),
                    "opentdf".to_string(),
                    "secure_mode".to_string(),
                ]),
                description: "Enable or disable local data security with OpenTDF encryption. \
                    This starts or stops a local OpenTDF platform stack (KAS + orchestrator OIDC) \
                    for attribute-based encryption of sensitive data."
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["enable", "disable", "status"],
                            "description": "Action to perform: enable (start stack), disable (stop stack), or status (check current state)",
                            "default": "enable"
                        },
                        "force": {
                            "type": "boolean",
                            "description": "Force restart even if stack is already running",
                            "default": false
                        }
                    },
                    "required": []
                }),
            },
        }
    }
}

impl Default for DataSecurityTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DataSecurityTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let action = args["action"].as_str().unwrap_or("enable");
        let force = args["force"].as_bool().unwrap_or(false);

        match action {
            "enable" => self.enable_security(force).await,
            "disable" => self.disable_security().await,
            "status" => self.check_status().await,
            _ => Err(crate::ToolError::InvalidParams(format!(
                "Unknown action: {action}. Use 'enable', 'disable', or 'status'"
            ))),
        }
    }
}

impl DataSecurityTool {
    async fn enable_security(&self, force: bool) -> crate::Result<Value> {
        info!("Enabling data security mode");

        // Check if stack is already running
        if let Some(stack) = OpenTdfStack::detect().await {
            if !force {
                let endpoints = stack.get_endpoints().await.map_err(|e| {
                    crate::ToolError::Execution(format!("Failed to get endpoints: {e}"))
                })?;

                return Ok(json!({
                    "success": true,
                    "message": "Data security already enabled",
                    "status": "running",
                    "endpoints": {
                        "kas_url": endpoints.kas_url,
                        "oauth_url": endpoints.oauth_url,
                        "token_endpoint": endpoints.token_endpoint
                    },
                    "hint": "Use force=true to restart the stack"
                }));
            }

            // Force restart - stop first
            info!("Force restart requested, stopping existing stack");
            if let Err(e) = stack.stop().await {
                warn!("Failed to stop existing stack: {e}");
            }
        }

        // Start the stack
        let stack = OpenTdfStack::new().map_err(|e| {
            crate::ToolError::Execution(format!("Failed to initialize stack: {e}"))
        })?;

        stack.start().await.map_err(|e| {
            crate::ToolError::Execution(format!("Failed to start OpenTDF stack: {e}"))
        })?;

        let endpoints = stack.get_endpoints().await.map_err(|e| {
            crate::ToolError::Execution(format!("Failed to get endpoints: {e}"))
        })?;

        Ok(json!({
            "success": true,
            "message": "Data security enabled successfully",
            "status": "running",
            "endpoints": {
                "kas_url": endpoints.kas_url,
                "oauth_url": endpoints.oauth_url,
                "token_endpoint": endpoints.token_endpoint,
                "client_id": endpoints.client_id
            },
            "usage": {
                "encrypt": "Use tdf_encrypt with --local flag or local KAS URL",
                "decrypt": "Use tdf_decrypt with --local flag (uses orchestrator OIDC)"
            }
        }))
    }

    async fn disable_security(&self) -> crate::Result<Value> {
        info!("Disabling data security mode");

        match OpenTdfStack::detect().await {
            Some(stack) => {
                stack.stop().await.map_err(|e| {
                    crate::ToolError::Execution(format!("Failed to stop stack: {e}"))
                })?;

                Ok(json!({
                    "success": true,
                    "message": "Data security disabled",
                    "status": "stopped"
                }))
            }
            None => Ok(json!({
                "success": true,
                "message": "Data security was not enabled",
                "status": "stopped"
            })),
        }
    }

    async fn check_status(&self) -> crate::Result<Value> {
        match OpenTdfStack::detect().await {
            Some(stack) => {
                let status = stack.status().await.map_err(|e| {
                    crate::ToolError::Execution(format!("Failed to get status: {e}"))
                })?;

                let status_str = match status {
                    StackStatus::Running => "running",
                    StackStatus::Partial => "partial",
                    StackStatus::Stopped => "stopped",
                    StackStatus::Unknown => "unknown",
                };

                if status == StackStatus::Running {
                    let endpoints = stack.get_endpoints().await.map_err(|e| {
                        crate::ToolError::Execution(format!("Failed to get endpoints: {e}"))
                    })?;

                    Ok(json!({
                        "enabled": true,
                        "status": status_str,
                        "endpoints": {
                            "kas_url": endpoints.kas_url,
                            "oauth_url": endpoints.oauth_url,
                            "token_endpoint": endpoints.token_endpoint,
                            "client_id": endpoints.client_id
                        }
                    }))
                } else {
                    Ok(json!({
                        "enabled": false,
                        "status": status_str,
                        "message": "Stack is not fully running"
                    }))
                }
            }
            None => {
                // Try to create stack to check runtime availability
                match OpenTdfStack::new() {
                    Ok(_) => Ok(json!({
                        "enabled": false,
                        "status": "stopped",
                        "container_runtime": "available",
                        "message": "Data security is not enabled. Use action='enable' to start."
                    })),
                    Err(e) => Ok(json!({
                        "enabled": false,
                        "status": "unavailable",
                        "error": e.to_string(),
                        "message": "No container runtime found. Install Docker, Podman, or Apple Container CLI."
                    })),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_schema() {
        let tool = DataSecurityTool::new();
        let schema = tool.schema();

        assert_eq!(schema.name, "enable_data_security");
        assert!(schema.description.contains("OpenTDF"));
        assert!(schema.aliases.as_ref().unwrap().contains(&"opentdf".to_string()));
    }

    #[tokio::test]
    async fn test_status_check() {
        let tool = DataSecurityTool::new();
        let result = tool.execute(json!({"action": "status"})).await;

        // Should succeed even if no stack is running
        assert!(result.is_ok());
        let value = result.unwrap();
        assert!(value.get("status").is_some());
    }

    #[tokio::test]
    async fn test_invalid_action() {
        let tool = DataSecurityTool::new();
        let result = tool.execute(json!({"action": "invalid"})).await;

        assert!(result.is_err());
    }
}
