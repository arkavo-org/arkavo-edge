use super::server::{Tool, ToolSchema};
use super::simulator_manager::SimulatorManager;
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub struct SimulatorControl {
    schema: ToolSchema,
    simulator_manager: Arc<Mutex<SimulatorManager>>,
}

impl SimulatorControl {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "simulator_control".to_string(),
                description: "Control iOS simulators - boot, shutdown, list devices".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["list", "boot", "shutdown", "refresh"],
                            "description": "Action to perform"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID for boot/shutdown actions"
                        }
                    },
                    "required": ["action"]
                }),
            },
            simulator_manager: Arc::new(Mutex::new(SimulatorManager::new())),
        }
    }
}

#[async_trait]
impl Tool for SimulatorControl {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        match action {
            "list" => {
                let manager = self
                    .simulator_manager
                    .lock()
                    .map_err(|_| TestError::Mcp("Failed to lock simulator manager".to_string()))?;

                let devices: Vec<_> = manager
                    .devices
                    .values()
                    .map(|d| {
                        serde_json::json!({
                            "udid": d.udid,
                            "name": d.name,
                            "state": d.state,
                            "device_type": d.device_type,
                            "runtime": d.runtime,
                            "is_available": d.is_available
                        })
                    })
                    .collect();

                Ok(serde_json::json!({
                    "devices": devices,
                    "count": devices.len()
                }))
            }
            "boot" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

                let manager = self
                    .simulator_manager
                    .lock()
                    .map_err(|_| TestError::Mcp("Failed to lock simulator manager".to_string()))?;

                match manager.boot_device(device_id) {
                    Ok(_) => Ok(serde_json::json!({
                        "success": true,
                        "message": format!("Device {} booted successfully", device_id)
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "success": false,
                        "error": e.to_string()
                    })),
                }
            }
            "shutdown" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

                let manager = self
                    .simulator_manager
                    .lock()
                    .map_err(|_| TestError::Mcp("Failed to lock simulator manager".to_string()))?;

                match manager.shutdown_device(device_id) {
                    Ok(_) => Ok(serde_json::json!({
                        "success": true,
                        "message": format!("Device {} shutdown successfully", device_id)
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "success": false,
                        "error": e.to_string()
                    })),
                }
            }
            "refresh" => {
                let mut manager = self
                    .simulator_manager
                    .lock()
                    .map_err(|_| TestError::Mcp("Failed to lock simulator manager".to_string()))?;

                match manager.refresh_devices() {
                    Ok(_) => Ok(serde_json::json!({
                        "success": true,
                        "message": "Device list refreshed"
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "success": false,
                        "error": e.to_string()
                    })),
                }
            }
            _ => Err(TestError::Mcp(format!("Unknown action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct AppManagement {
    schema: ToolSchema,
    simulator_manager: Arc<Mutex<SimulatorManager>>,
}

impl AppManagement {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "app_management".to_string(),
                description: "Manage iOS apps - install, uninstall, launch, terminate".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["install", "uninstall", "launch", "terminate", "list"],
                            "description": "Action to perform"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID"
                        },
                        "app_path": {
                            "type": "string",
                            "description": "Path to .app bundle for install action"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "Bundle ID for uninstall/launch/terminate actions"
                        },
                        "arguments": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Launch arguments for the app"
                        }
                    },
                    "required": ["action", "device_id"]
                }),
            },
            simulator_manager: Arc::new(Mutex::new(SimulatorManager::new())),
        }
    }
}

#[async_trait]
impl Tool for AppManagement {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        let device_id = params
            .get("device_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

        let manager = self
            .simulator_manager
            .lock()
            .map_err(|_| TestError::Mcp("Failed to lock simulator manager".to_string()))?;

        match action {
            "install" => {
                let app_path = params
                    .get("app_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing app_path parameter".to_string()))?;

                match manager.install_app(device_id, app_path) {
                    Ok(_) => Ok(serde_json::json!({
                        "success": true,
                        "message": format!("App installed successfully on device {}", device_id)
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "success": false,
                        "error": e.to_string()
                    })),
                }
            }
            "uninstall" => {
                let bundle_id = params
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing bundle_id parameter".to_string()))?;

                match manager.uninstall_app(device_id, bundle_id) {
                    Ok(_) => Ok(serde_json::json!({
                        "success": true,
                        "message": format!("App {} uninstalled successfully", bundle_id)
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "success": false,
                        "error": e.to_string()
                    })),
                }
            }
            "launch" => {
                let bundle_id = params
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing bundle_id parameter".to_string()))?;

                let args: Vec<&str> = if let Some(arguments) = params.get("arguments") {
                    arguments
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default()
                } else {
                    vec![]
                };

                // Log the launch attempt
                eprintln!("MCP: Attempting to launch app {} on device {}", bundle_id, device_id);
                
                match manager.launch_app(device_id, bundle_id, &args) {
                    Ok(pid) => {
                        eprintln!("MCP: App launched successfully with PID: {}", pid);
                        Ok(serde_json::json!({
                            "success": true,
                            "message": format!("App {} launched successfully", bundle_id),
                            "pid": pid,
                            "device_id": device_id,
                            "bundle_id": bundle_id
                        }))
                    },
                    Err(e) => {
                        eprintln!("MCP: Failed to launch app: {}", e);
                        Ok(serde_json::json!({
                            "success": false,
                            "error": e.to_string(),
                            "device_id": device_id,
                            "bundle_id": bundle_id,
                            "suggestion": "Ensure the app is installed and the bundle ID is correct"
                        }))
                    }
                }
            }
            "terminate" => {
                let bundle_id = params
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing bundle_id parameter".to_string()))?;

                match manager.terminate_app(device_id, bundle_id) {
                    Ok(_) => Ok(serde_json::json!({
                        "success": true,
                        "message": format!("App {} terminated successfully", bundle_id)
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "success": false,
                        "error": e.to_string()
                    })),
                }
            }
            "list" => match manager.list_apps(device_id) {
                Ok(apps) => Ok(serde_json::json!({
                    "success": true,
                    "apps": apps,
                    "count": apps.len()
                })),
                Err(e) => Ok(serde_json::json!({
                    "success": false,
                    "error": e.to_string()
                })),
            },
            _ => Err(TestError::Mcp(format!("Unknown action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct FileOperations {
    schema: ToolSchema,
    simulator_manager: Arc<Mutex<SimulatorManager>>,
}

impl FileOperations {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "file_operations".to_string(),
                description: "Transfer files to/from iOS simulator".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["push", "pull", "get_container"],
                            "description": "Action to perform"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID"
                        },
                        "local_path": {
                            "type": "string",
                            "description": "Local file path"
                        },
                        "remote_path": {
                            "type": "string",
                            "description": "Remote file path on device"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "Bundle ID for get_container action"
                        }
                    },
                    "required": ["action", "device_id"]
                }),
            },
            simulator_manager: Arc::new(Mutex::new(SimulatorManager::new())),
        }
    }
}

#[async_trait]
impl Tool for FileOperations {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        let device_id = params
            .get("device_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

        let manager = self
            .simulator_manager
            .lock()
            .map_err(|_| TestError::Mcp("Failed to lock simulator manager".to_string()))?;

        match action {
            "push" => {
                let local_path = params
                    .get("local_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing local_path parameter".to_string()))?;

                let remote_path = params
                    .get("remote_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing remote_path parameter".to_string()))?;

                match manager.push_file(device_id, local_path, remote_path) {
                    Ok(_) => Ok(serde_json::json!({
                        "success": true,
                        "message": format!("File pushed successfully to {}", remote_path)
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "success": false,
                        "error": e.to_string()
                    })),
                }
            }
            "pull" => {
                let remote_path = params
                    .get("remote_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing remote_path parameter".to_string()))?;

                let local_path = params
                    .get("local_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing local_path parameter".to_string()))?;

                match manager.pull_file(device_id, remote_path, local_path) {
                    Ok(_) => Ok(serde_json::json!({
                        "success": true,
                        "message": format!("File pulled successfully to {}", local_path)
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "success": false,
                        "error": e.to_string()
                    })),
                }
            }
            "get_container" => {
                let bundle_id = params
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing bundle_id parameter".to_string()))?;

                match manager.get_app_container(device_id, bundle_id) {
                    Ok(path) => Ok(serde_json::json!({
                        "success": true,
                        "container_path": path
                    })),
                    Err(e) => Ok(serde_json::json!({
                        "success": false,
                        "error": e.to_string()
                    })),
                }
            }
            _ => Err(TestError::Mcp(format!("Unknown action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}