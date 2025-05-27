use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct DeviceManagementKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl DeviceManagementKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "device_management".to_string(),
                description: "Manage iOS devices and simulators for testing".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": [
                                "list", "list_booted", "get_active", "set_active",
                                "boot", "shutdown", "create", "delete", "refresh"
                            ],
                            "description": "Device management action"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Device identifier (UUID)"
                        },
                        "name": {
                            "type": "string",
                            "description": "Device name (for create action)"
                        },
                        "device_type": {
                            "type": "string",
                            "description": "Device type identifier (e.g., 'iPhone 15 Pro')"
                        },
                        "runtime": {
                            "type": "string",
                            "description": "iOS runtime version (e.g., 'iOS 17.2')"
                        }
                    },
                    "required": ["action"]
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for DeviceManagementKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        match action {
            "list" => {
                let devices = self.device_manager.get_all_devices();
                Ok(json!({
                    "devices": devices,
                    "count": devices.len()
                }))
            }
            
            "list_booted" => {
                let devices = self.device_manager.get_booted_devices();
                Ok(json!({
                    "devices": devices,
                    "count": devices.len()
                }))
            }
            
            "get_active" => {
                match self.device_manager.get_active_device() {
                    Some(device) => Ok(json!({
                        "device": device,
                        "active": true
                    })),
                    None => Ok(json!({
                        "device": null,
                        "active": false,
                        "message": "No active device set"
                    }))
                }
            }
            
            "set_active" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;
                
                self.device_manager.set_active_device(device_id)?;
                
                Ok(json!({
                    "success": true,
                    "device_id": device_id,
                    "message": "Active device updated"
                }))
            }
            
            "boot" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;
                
                self.device_manager.boot_device(device_id)?;
                
                Ok(json!({
                    "success": true,
                    "device_id": device_id,
                    "message": "Device booted successfully"
                }))
            }
            
            "shutdown" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;
                
                self.device_manager.shutdown_device(device_id)?;
                
                Ok(json!({
                    "success": true,
                    "device_id": device_id,
                    "message": "Device shutdown successfully"
                }))
            }
            
            "create" => {
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing name parameter".to_string()))?;
                
                let device_type = params
                    .get("device_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("com.apple.CoreSimulator.SimDeviceType.iPhone-15-Pro");
                
                let runtime = params
                    .get("runtime")
                    .and_then(|v| v.as_str())
                    .unwrap_or("com.apple.CoreSimulator.SimRuntime.iOS-17-2");
                
                let device_id = self.device_manager.create_device(name, device_type, runtime)?;
                
                Ok(json!({
                    "success": true,
                    "device_id": device_id,
                    "name": name,
                    "message": "Device created successfully"
                }))
            }
            
            "delete" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;
                
                self.device_manager.delete_device(device_id)?;
                
                Ok(json!({
                    "success": true,
                    "device_id": device_id,
                    "message": "Device deleted successfully"
                }))
            }
            
            "refresh" => {
                let devices = self.device_manager.refresh_devices()?;
                Ok(json!({
                    "success": true,
                    "devices": devices,
                    "count": devices.len(),
                    "message": "Device list refreshed"
                }))
            }
            
            _ => Err(TestError::Mcp(format!("Unknown action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}