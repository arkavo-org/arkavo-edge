use super::device_boot_manager::DeviceBootManager;
use super::device_health_manager::DeviceHealthManager;
use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

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
                                "list", "list_all", "list_booted", "get_active", "set_active",
                                "boot_wait", "boot_status", "shutdown", 
                                "create", "delete", "refresh", "health_check", "cleanup_unhealthy"
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
                        },
                        "timeout_seconds": {
                            "type": "number",
                            "description": "Timeout for boot_wait operation (default: 60)",
                            "default": 60
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "For cleanup_unhealthy: only show what would be deleted",
                            "default": false
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
                    "count": devices.len(),
                    "note": "Only showing healthy devices. Use 'list_all' to see all devices including unavailable ones."
                }))
            }
            
            "list_all" => {
                // Force refresh to get all devices including unhealthy ones
                let devices = self.device_manager.refresh_devices_all()?;
                Ok(json!({
                    "devices": devices,
                    "count": devices.len(),
                    "note": "Showing all devices including unavailable ones. Use 'cleanup_unhealthy' to remove bad devices."
                }))
            }

            "list_booted" => {
                let devices = self.device_manager.get_booted_devices();
                Ok(json!({
                    "devices": devices,
                    "count": devices.len()
                }))
            }

            "get_active" => match self.device_manager.get_active_device() {
                Some(device) => Ok(json!({
                    "device": device,
                    "active": true
                })),
                None => Ok(json!({
                    "device": null,
                    "active": false,
                    "message": "No active device set"
                })),
            },

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

            "boot_wait" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;
                    
                let timeout_secs = params
                    .get("timeout_seconds")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(60.0);
                    
                let timeout = Duration::from_secs_f64(timeout_secs);
                
                eprintln!("[DeviceManagement] Starting boot_wait for device {} with timeout {:?}", device_id, timeout);
                
                let boot_status = DeviceBootManager::boot_device_with_wait(device_id, timeout).await?;
                
                // Refresh device list to get updated state
                self.device_manager.refresh_devices()?;
                
                Ok(json!({
                    "success": boot_status.current_state == super::device_boot_manager::BootState::Ready,
                    "device_id": device_id,
                    "boot_status": boot_status,
                    "message": match boot_status.current_state {
                        super::device_boot_manager::BootState::Ready => "Device booted and ready",
                        super::device_boot_manager::BootState::Failed => "Device boot failed",
                        _ => "Device boot incomplete"
                    }
                }))
            }
            
            "boot_status" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;
                    
                let progress = DeviceBootManager::get_boot_progress(device_id).await?;
                
                Ok(json!({
                    "device_id": device_id,
                    "progress": progress,
                    "hint": "Use boot_wait to boot and wait for device to be ready"
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

                let device_id = self
                    .device_manager
                    .create_device(name, device_type, runtime)?;

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
                    "message": "Device list refreshed (healthy devices only)"
                }))
            }
            
            "health_check" => {
                let health_reports = DeviceHealthManager::check_all_devices_health()?;
                let unhealthy_count = health_reports.iter().filter(|r| !r.is_healthy).count();
                let healthy_count = health_reports.iter().filter(|r| r.is_healthy).count();
                
                Ok(json!({
                    "success": true,
                    "total_devices": health_reports.len(),
                    "healthy_devices": healthy_count,
                    "unhealthy_devices": unhealthy_count,
                    "health_reports": health_reports,
                    "recommendation": if unhealthy_count > 0 {
                        "Run 'cleanup_unhealthy' to remove devices with missing runtimes"
                    } else {
                        "All devices are healthy"
                    }
                }))
            }
            
            "cleanup_unhealthy" => {
                let dry_run = params
                    .get("dry_run")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                    
                eprintln!("[DeviceManagement] Running cleanup_unhealthy (dry_run: {})", dry_run);
                
                // First try the built-in simctl command
                if !dry_run {
                    if let Err(e) = DeviceHealthManager::delete_unavailable_devices() {
                        eprintln!("Warning: simctl delete unavailable failed: {}", e);
                    }
                }
                
                // Then do our own cleanup
                let deleted_devices = DeviceHealthManager::delete_unhealthy_devices(dry_run)?;
                
                // Refresh device list after cleanup
                if !dry_run && !deleted_devices.is_empty() {
                    self.device_manager.refresh_devices()?;
                }
                
                Ok(json!({
                    "success": true,
                    "dry_run": dry_run,
                    "deleted_devices": deleted_devices,
                    "count": deleted_devices.len(),
                    "message": if dry_run {
                        format!("Would delete {} unhealthy devices", deleted_devices.len())
                    } else {
                        format!("Deleted {} unhealthy devices", deleted_devices.len())
                    }
                }))
            }

            _ => Err(TestError::Mcp(format!("Unknown action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
