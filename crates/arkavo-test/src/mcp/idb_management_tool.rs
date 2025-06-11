use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{json, Value};

#[cfg(target_os = "macos")]
use super::idb_installer::IdbInstaller;
#[cfg(target_os = "macos")]
use super::idb_recovery::IdbRecovery;
#[cfg(target_os = "macos")]
use super::idb_wrapper::IdbWrapper;

pub struct IdbManagementTool {
    schema: ToolSchema,
    #[cfg(target_os = "macos")]
    recovery: IdbRecovery,
}

impl IdbManagementTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "idb_management".to_string(),
                description: "Manage IDB companion health, status, and recovery operations".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["health_check", "recover", "status", "list_targets", "install"],
                            "description": "Action to perform"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID for device-specific operations"
                        }
                    },
                    "required": ["action"]
                }),
            },
            #[cfg(target_os = "macos")]
            recovery: IdbRecovery::new(),
        }
    }
}

#[async_trait]
impl Tool for IdbManagementTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params["action"].as_str()
            .ok_or_else(|| TestError::Mcp("action is required".to_string()))?;
        
        #[cfg(not(target_os = "macos"))]
        {
            return Ok(json!({
                "success": false,
                "error": {
                    "code": "PLATFORM_NOT_SUPPORTED",
                    "message": "IDB management is only supported on macOS"
                }
            }));
        }
        
        #[cfg(target_os = "macos")]
        {
            match action {
                "health_check" => {
                    let is_running = IdbRecovery::is_companion_running().await;
                    let can_list = match IdbWrapper::list_targets().await {
                        Ok(targets) => !targets.as_array().map(|a| a.is_empty()).unwrap_or(true),
                        Err(_) => false,
                    };
                    
                    let device_responsive = if let Some(device_id) = params["device_id"].as_str() {
                        Some(IdbRecovery::check_device_responsive(device_id).await)
                    } else {
                        None
                    };
                    
                    Ok(json!({
                        "success": true,
                        "idb_health": {
                            "companion_process_running": is_running,
                            "can_list_targets": can_list,
                            "device_responsive": device_responsive,
                            "overall_status": if is_running && can_list { "healthy" } else { "unhealthy" }
                        },
                        "recommendation": if !is_running || !can_list {
                            "IDB companion appears unhealthy. Consider running 'recover' action."
                        } else {
                            "IDB companion is functioning normally."
                        }
                    }))
                }
                
                "recover" => {
                    match self.recovery.attempt_recovery().await {
                        Ok(_) => {
                            // Check if recovery was successful
                            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                            let is_running = IdbRecovery::is_companion_running().await;
                            
                            Ok(json!({
                                "success": true,
                                "message": "IDB recovery process completed",
                                "companion_running_after_recovery": is_running,
                                "next_steps": [
                                    "Wait a few seconds for IDB to fully initialize",
                                    "Run 'health_check' to verify recovery success",
                                    "Retry your previous operation"
                                ]
                            }))
                        }
                        Err(e) => Ok(json!({
                            "success": false,
                            "error": {
                                "code": "RECOVERY_FAILED",
                                "message": e.to_string()
                            }
                        }))
                    }
                }
                
                "status" => {
                    match IdbWrapper::list_targets().await {
                        Ok(targets) => {
                            let target_count = targets.as_array()
                                .map(|a| a.len())
                                .unwrap_or(0);
                            
                            Ok(json!({
                                "success": true,
                                "idb_status": {
                                    "companion_running": IdbRecovery::is_companion_running().await,
                                    "target_count": target_count,
                                    "targets": targets
                                }
                            }))
                        }
                        Err(e) => Ok(json!({
                            "success": false,
                            "error": {
                                "code": "STATUS_CHECK_FAILED",
                                "message": e.to_string()
                            },
                            "companion_running": IdbRecovery::is_companion_running().await
                        }))
                    }
                }
                
                "list_targets" => {
                    match IdbWrapper::list_targets().await {
                        Ok(targets) => Ok(json!({
                            "success": true,
                            "targets": targets
                        })),
                        Err(e) => Ok(json!({
                            "success": false,
                            "error": {
                                "code": "LIST_TARGETS_FAILED",
                                "message": e.to_string()
                            }
                        }))
                    }
                }
                
                "install" => {
                    if IdbInstaller::is_idb_installed() {
                        Ok(json!({
                            "success": true,
                            "message": "IDB is already installed",
                            "idb_path": IdbInstaller::get_idb_path()
                        }))
                    } else {
                        match IdbInstaller::attempt_auto_install().await {
                            Ok(message) => Ok(json!({
                                "success": true,
                                "message": message,
                                "installed": IdbInstaller::is_idb_installed(),
                                "idb_path": IdbInstaller::get_idb_path()
                            })),
                            Err(e) => Ok(json!({
                                "success": false,
                                "error": {
                                    "code": "INSTALL_FAILED",
                                    "message": e.to_string()
                                },
                                "instructions": IdbInstaller::get_install_instructions()
                            }))
                        }
                    }
                }
                
                _ => Ok(json!({
                    "success": false,
                    "error": {
                        "code": "UNKNOWN_ACTION",
                        "message": format!("Unknown action: {}", action)
                    }
                }))
            }
        }
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

impl Default for IdbManagementTool {
    fn default() -> Self {
        Self::new()
    }
}