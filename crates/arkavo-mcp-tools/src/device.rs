use crate::server::Tool;
use crate::{Result, ToolError};
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::process::Command;

/// Physical device information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub udid: String,
    pub name: String,
    pub model: String,
    pub os_version: String,
    pub connection_type: String,
}

/// Execute devicectl command (Xcode 15+)
async fn run_devicectl(args: &[&str]) -> Result<String> {
    let output = Command::new("xcrun")
        .arg("devicectl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| ToolError::Execution(format!("Failed to run devicectl: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::Execution(format!("devicectl failed: {}", stderr)));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

// ============================================================================
// DeviceListTool - List connected physical devices
// ============================================================================

pub struct DeviceListTool {
    schema: ToolSchema,
}

impl DeviceListTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_list".to_string(),
                aliases: Some(vec!["list_devices".to_string(), "devices".to_string()]),
                description: "List connected physical Apple devices (iPhone, iPad, Apple Watch, etc.).".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "available_only": {
                            "type": "boolean",
                            "description": "Only show available devices (default: true)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for DeviceListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceListTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let available_only = params
            .get("available_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        // Try devicectl first (Xcode 15+)
        let output = run_devicectl(&["list", "devices", "--json-output", "/dev/stdout"]).await;

        match output {
            Ok(json_str) => {
                // Parse JSON output from devicectl
                let json_output: Value = serde_json::from_str(&json_str).unwrap_or(json!({}));

                let devices: Vec<DeviceInfo> = json_output
                    .get("result")
                    .and_then(|r| r.get("devices"))
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|d| {
                                let identifier = d
                                    .get("hardwareProperties")
                                    .and_then(|h| h.get("udid"))
                                    .and_then(|u| u.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let name = d.get("name").and_then(|n| n.as_str()).unwrap_or("Unknown").to_string();
                                let model = d
                                    .get("hardwareProperties")
                                    .and_then(|h| h.get("marketingName"))
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("Unknown")
                                    .to_string();
                                let os_version = d
                                    .get("deviceProperties")
                                    .and_then(|dp| dp.get("osVersionNumber"))
                                    .and_then(|o| o.as_str())
                                    .unwrap_or("Unknown")
                                    .to_string();
                                let connection_type = d
                                    .get("connectionProperties")
                                    .and_then(|c| c.get("transportType"))
                                    .and_then(|t| t.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();

                                if identifier.is_empty() {
                                    return None;
                                }

                                Some(DeviceInfo {
                                    udid: identifier,
                                    name,
                                    model,
                                    os_version,
                                    connection_type,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let filtered: Vec<_> = if available_only {
                    devices.into_iter().filter(|d| !d.udid.is_empty()).collect()
                } else {
                    devices
                };

                Ok(json!({
                    "success": true,
                    "count": filtered.len(),
                    "devices": filtered
                }))
            }
            Err(_) => {
                // Fallback to instruments or system_profiler
                let output = Command::new("system_profiler")
                    .args(["SPUSBDataType", "-json"])
                    .output()
                    .await
                    .map_err(|e| ToolError::Execution(format!("Failed to list devices: {}", e)))?;

                let stdout = String::from_utf8_lossy(&output.stdout);

                Ok(json!({
                    "success": true,
                    "message": "devicectl not available, using fallback",
                    "raw_output": stdout.to_string()
                }))
            }
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceInstallAppTool - Install app on device
// ============================================================================

pub struct DeviceInstallAppTool {
    schema: ToolSchema,
}

impl DeviceInstallAppTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_install_app".to_string(),
                aliases: Some(vec!["install_app".to_string()]),
                description: "Install an app (.app or .ipa) on a physical device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID"
                        },
                        "app_path": {
                            "type": "string",
                            "description": "Path to .app bundle or .ipa file"
                        }
                    },
                    "required": ["device_id", "app_path"]
                }),
            },
        }
    }
}

impl Default for DeviceInstallAppTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceInstallAppTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let device_id = params.get("device_id").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'device_id' is required".to_string())
        })?;

        let app_path = params.get("app_path").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'app_path' is required".to_string())
        })?;

        let output = run_devicectl(&[
            "device",
            "install",
            "app",
            "--device",
            device_id,
            app_path,
        ])
        .await?;

        Ok(json!({
            "success": true,
            "message": "App installed successfully",
            "device_id": device_id,
            "app_path": app_path,
            "output": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceLaunchAppTool - Launch app on device
// ============================================================================

pub struct DeviceLaunchAppTool {
    schema: ToolSchema,
}

impl DeviceLaunchAppTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_launch_app".to_string(),
                aliases: Some(vec!["launch_app".to_string()]),
                description: "Launch an app on a physical device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "App bundle identifier"
                        },
                        "arguments": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Launch arguments"
                        }
                    },
                    "required": ["device_id", "bundle_id"]
                }),
            },
        }
    }
}

impl Default for DeviceLaunchAppTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceLaunchAppTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let device_id = params.get("device_id").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'device_id' is required".to_string())
        })?;

        let bundle_id = params.get("bundle_id").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'bundle_id' is required".to_string())
        })?;

        let mut args = vec![
            "device",
            "process",
            "launch",
            "--device",
            device_id,
            bundle_id,
        ];

        let launch_args: Vec<String>;
        if let Some(arguments) = params.get("arguments").and_then(|v| v.as_array()) {
            launch_args = arguments
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            for arg in &launch_args {
                args.push(arg);
            }
        }

        let output = run_devicectl(&args).await?;

        Ok(json!({
            "success": true,
            "message": "App launched",
            "device_id": device_id,
            "bundle_id": bundle_id,
            "output": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// DeviceStopAppTool - Stop app on device
// ============================================================================

pub struct DeviceStopAppTool {
    schema: ToolSchema,
}

impl DeviceStopAppTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "device_stop_app".to_string(),
                aliases: Some(vec!["stop_app".to_string()]),
                description: "Stop a running app on a physical device.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Device UDID"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "App bundle identifier"
                        }
                    },
                    "required": ["device_id", "bundle_id"]
                }),
            },
        }
    }
}

impl Default for DeviceStopAppTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DeviceStopAppTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let device_id = params.get("device_id").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'device_id' is required".to_string())
        })?;

        let bundle_id = params.get("bundle_id").and_then(|v| v.as_str()).ok_or_else(|| {
            ToolError::InvalidParams("'bundle_id' is required".to_string())
        })?;

        // First get the PID of the running app
        let list_output = run_devicectl(&[
            "device",
            "info",
            "processes",
            "--device",
            device_id,
            "--json-output",
            "/dev/stdout",
        ])
        .await;

        match list_output {
            Ok(json_str) => {
                let json_output: Value = serde_json::from_str(&json_str).unwrap_or(json!({}));

                // Find process with matching bundle ID and terminate it
                if let Some(processes) = json_output
                    .get("result")
                    .and_then(|r| r.get("runningProcesses"))
                    .and_then(|p| p.as_array())
                {
                    for process in processes {
                        if process.get("bundleID").and_then(|b| b.as_str()) == Some(bundle_id) {
                            if let Some(pid) = process.get("processIdentifier").and_then(|p| p.as_i64()) {
                                let pid_str = pid.to_string();
                                let _ = run_devicectl(&[
                                    "device",
                                    "process",
                                    "signal",
                                    "--device",
                                    device_id,
                                    "--pid",
                                    &pid_str,
                                    "--signal",
                                    "SIGTERM",
                                ])
                                .await;

                                return Ok(json!({
                                    "success": true,
                                    "message": "App stopped",
                                    "device_id": device_id,
                                    "bundle_id": bundle_id,
                                    "pid": pid
                                }));
                            }
                        }
                    }
                }

                Ok(json!({
                    "success": false,
                    "message": "App not found running on device",
                    "device_id": device_id,
                    "bundle_id": bundle_id
                }))
            }
            Err(e) => Err(e),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_list_tool_schema() {
        let tool = DeviceListTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_list");
        assert!(schema.aliases.as_ref().unwrap().contains(&"devices".to_string()));
    }

    #[test]
    fn test_device_install_app_tool_schema() {
        let tool = DeviceInstallAppTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_install_app");
    }

    #[test]
    fn test_device_launch_app_tool_schema() {
        let tool = DeviceLaunchAppTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_launch_app");
    }

    #[test]
    fn test_device_stop_app_tool_schema() {
        let tool = DeviceStopAppTool::new();
        let schema = tool.schema();
        assert_eq!(schema.name, "device_stop_app");
    }
}
