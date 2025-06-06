use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatorProcess {
    pub pid: u32,
    pub name: String,
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatorDiagnostics {
    pub device_id: String,
    pub runtime_path: Option<String>,
    pub data_path: Option<String>,
    pub running_processes: Vec<SimulatorProcess>,
    pub memory_usage_mb: Option<f64>,
    pub disk_usage_mb: Option<f64>,
}

pub struct SimulatorAdvancedKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl SimulatorAdvancedKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "simulator_advanced".to_string(),
                description: "Advanced simulator management including diagnostics, process management, and system information".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": [
                                "diagnostics", "list_processes", "kill_process",
                                "reset_data", "clone_device", "get_runtime_info",
                                "list_apps", "uninstall_app", "clear_keychain"
                            ],
                            "description": "Advanced simulator action"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Device identifier (UUID)"
                        },
                        "process_name": {
                            "type": "string",
                            "description": "Process name to kill"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "App bundle identifier"
                        },
                        "new_name": {
                            "type": "string",
                            "description": "Name for cloned device"
                        }
                    },
                    "required": ["action"]
                }),
            },
            device_manager,
        }
    }

    async fn get_diagnostics(&self, device_id: &str) -> Result<SimulatorDiagnostics> {
        // Get device runtime and data paths
        let path_output = Command::new("xcrun")
            .args(["simctl", "get_app_container", device_id, "booted"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to get device paths: {}", e)))?;

        let data_path = if path_output.status.success() {
            Some(
                String::from_utf8_lossy(&path_output.stdout)
                    .trim()
                    .to_string(),
            )
        } else {
            None
        };

        // Get running processes
        let processes = self.list_device_processes(device_id).await?;

        // Get memory usage (approximate)
        let memory_usage_mb = None; // Would need more complex implementation

        // Get disk usage
        let disk_usage_mb = if let Some(ref path) = data_path {
            self.get_directory_size_mb(path).ok()
        } else {
            None
        };

        Ok(SimulatorDiagnostics {
            device_id: device_id.to_string(),
            runtime_path: None, // Would need to parse from device info
            data_path,
            running_processes: processes,
            memory_usage_mb,
            disk_usage_mb,
        })
    }

    async fn list_device_processes(&self, device_id: &str) -> Result<Vec<SimulatorProcess>> {
        let output = Command::new("xcrun")
            .args(["simctl", "spawn", device_id, "launchctl", "list"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list processes: {}", e)))?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let process_list = String::from_utf8_lossy(&output.stdout);
        let mut processes = Vec::new();

        for line in process_list.lines().skip(1) {
            // Skip header
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                if let Ok(pid) = parts[0].parse::<u32>() {
                    let name = parts[2].to_string();
                    let bundle_id = if name.contains('.') {
                        Some(name.clone())
                    } else {
                        None
                    };

                    processes.push(SimulatorProcess {
                        pid,
                        name,
                        bundle_id,
                    });
                }
            }
        }

        Ok(processes)
    }

    fn get_directory_size_mb(&self, path: &str) -> Result<f64> {
        let output = Command::new("du")
            .args(["-sk", path])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to get directory size: {}", e)))?;

        if output.status.success() {
            let size_str = String::from_utf8_lossy(&output.stdout);
            if let Some(size_kb) = size_str.split_whitespace().next() {
                if let Ok(kb) = size_kb.parse::<f64>() {
                    return Ok(kb / 1024.0); // Convert KB to MB
                }
            }
        }

        Ok(0.0)
    }
}

#[async_trait]
impl Tool for SimulatorAdvancedKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        match action {
            "diagnostics" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

                let diagnostics = self.get_diagnostics(device_id).await?;

                Ok(json!({
                    "success": true,
                    "diagnostics": diagnostics
                }))
            }

            "list_processes" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

                let processes = self.list_device_processes(device_id).await?;

                Ok(json!({
                    "success": true,
                    "device_id": device_id,
                    "processes": processes,
                    "count": processes.len()
                }))
            }

            "kill_process" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

                let process_name = params
                    .get("process_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing process_name parameter".to_string()))?;

                let output = Command::new("xcrun")
                    .args(["simctl", "terminate", device_id, process_name])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to kill process: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "device_id": device_id,
                    "process": process_name,
                    "message": if output.status.success() {
                        "Process terminated"
                    } else {
                        "Failed to terminate process"
                    }
                }))
            }

            "reset_data" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

                let output = Command::new("xcrun")
                    .args(["simctl", "erase", device_id])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to reset device: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "device_id": device_id,
                    "message": if output.status.success() {
                        "Device data erased successfully"
                    } else {
                        "Failed to erase device data"
                    }
                }))
            }

            "clone_device" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

                let new_name = params
                    .get("new_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing new_name parameter".to_string()))?;

                let output = Command::new("xcrun")
                    .args(["simctl", "clone", device_id, new_name])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to clone device: {}", e)))?;

                if output.status.success() {
                    let new_device_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

                    // Refresh devices to include the clone
                    self.device_manager.refresh_devices()?;

                    Ok(json!({
                        "success": true,
                        "original_device_id": device_id,
                        "cloned_device_id": new_device_id,
                        "name": new_name,
                        "message": "Device cloned successfully"
                    }))
                } else {
                    Ok(json!({
                        "success": false,
                        "error": String::from_utf8_lossy(&output.stderr).to_string()
                    }))
                }
            }

            "list_apps" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

                let output = Command::new("xcrun")
                    .args(["simctl", "listapps", device_id])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to list apps: {}", e)))?;

                if output.status.success() {
                    let apps_json = String::from_utf8_lossy(&output.stdout);
                    if let Ok(apps_data) = serde_json::from_str::<Value>(&apps_json) {
                        Ok(json!({
                            "success": true,
                            "device_id": device_id,
                            "apps": apps_data
                        }))
                    } else {
                        Ok(json!({
                            "success": false,
                            "error": "Failed to parse app list"
                        }))
                    }
                } else {
                    Ok(json!({
                        "success": false,
                        "error": String::from_utf8_lossy(&output.stderr).to_string()
                    }))
                }
            }

            "uninstall_app" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

                let bundle_id = params
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing bundle_id parameter".to_string()))?;

                let output = Command::new("xcrun")
                    .args(["simctl", "uninstall", device_id, bundle_id])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to uninstall app: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "device_id": device_id,
                    "bundle_id": bundle_id,
                    "message": if output.status.success() {
                        "App uninstalled successfully"
                    } else {
                        "Failed to uninstall app"
                    }
                }))
            }

            "clear_keychain" => {
                let device_id = params
                    .get("device_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing device_id parameter".to_string()))?;

                let output = Command::new("xcrun")
                    .args(["simctl", "keychain", device_id, "reset"])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to clear keychain: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "device_id": device_id,
                    "message": if output.status.success() {
                        "Keychain cleared successfully"
                    } else {
                        "Failed to clear keychain"
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
