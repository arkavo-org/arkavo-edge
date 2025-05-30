use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;

pub struct DeepLinkKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl DeepLinkKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "deep_link".to_string(),
                description:
                    "Open deep links or URLs in iOS apps to navigate directly to specific screens"
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "The deep link URL or universal link (e.g., 'myapp://profile', 'https://example.com/path')"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "Optional bundle ID to launch a specific app before opening the link"
                        }
                    },
                    "required": ["url"]
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for DeepLinkKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing url parameter".to_string()))?;

        // Get device ID
        let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else {
            match self.device_manager.get_active_device() {
                Some(device) => device.id,
                None => {
                    self.device_manager.refresh_devices().ok();
                    match self.device_manager.get_booted_devices().first() {
                        Some(device) => device.id.clone(),
                        None => {
                            return Ok(serde_json::json!({
                                "error": {
                                    "code": "NO_BOOTED_DEVICE",
                                    "message": "No booted iOS device found"
                                }
                            }));
                        }
                    }
                }
            }
        };

        // Launch specific app if bundle ID is provided
        if let Some(bundle_id) = params.get("bundle_id").and_then(|v| v.as_str()) {
            let launch_output = Command::new("xcrun")
                .args(["simctl", "launch", &device_id, bundle_id])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to launch app: {}", e)))?;

            if !launch_output.status.success() {
                return Ok(json!({
                    "success": false,
                    "error": {
                        "message": "Failed to launch app",
                        "details": String::from_utf8_lossy(&launch_output.stderr).trim().to_string(),
                        "bundle_id": bundle_id
                    }
                }));
            }

            // Give the app a moment to launch
            std::thread::sleep(std::time::Duration::from_millis(500));
        }

        // Open the deep link
        let output = Command::new("xcrun")
            .args(["simctl", "openurl", &device_id, url])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to open URL: {}", e)))?;

        if output.status.success() {
            Ok(json!({
                "success": true,
                "action": "deep_link",
                "url": url,
                "device_id": device_id,
                "bundle_id": params.get("bundle_id"),
                "message": "Deep link opened successfully"
            }))
        } else {
            let error_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();

            // Provide helpful error messages
            let suggestion = if error_msg.contains("No app registered") {
                "The URL scheme is not registered by any app. Make sure the app is installed and supports this URL scheme."
            } else if error_msg.contains("Invalid URL") {
                "The URL format is invalid. Check that it's properly formatted (e.g., 'myapp://path' or 'https://example.com')."
            } else {
                "Make sure the app is installed and the URL scheme is properly configured in the app's Info.plist."
            };

            Ok(json!({
                "success": false,
                "error": {
                    "message": error_msg,
                    "suggestion": suggestion,
                    "url": url,
                    "device_id": device_id
                }
            }))
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct AppLauncherKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl AppLauncherKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "app_launcher".to_string(),
                description: "Launch, terminate, or get info about iOS apps".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["launch", "terminate", "install", "uninstall", "list", "info"],
                            "description": "Action to perform"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "Bundle identifier of the app (e.g., 'com.example.app')"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "app_path": {
                            "type": "string",
                            "description": "Path to .app bundle (for install action)"
                        },
                        "launch_args": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Launch arguments to pass to the app"
                        },
                        "env": {
                            "type": "object",
                            "description": "Environment variables to set when launching"
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
impl Tool for AppLauncherKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        // Get device ID
        let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else {
            match self.device_manager.get_active_device() {
                Some(device) => device.id,
                None => {
                    self.device_manager.refresh_devices().ok();
                    match self.device_manager.get_booted_devices().first() {
                        Some(device) => device.id.clone(),
                        None => {
                            return Ok(serde_json::json!({
                                "error": {
                                    "code": "NO_BOOTED_DEVICE",
                                    "message": "No booted iOS device found"
                                }
                            }));
                        }
                    }
                }
            }
        };

        match action {
            "launch" => {
                let bundle_id = params
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing bundle_id for launch".to_string()))?;

                let mut cmd = Command::new("xcrun");
                cmd.args(["simctl", "launch", &device_id, bundle_id]);

                // Add launch arguments if provided
                if let Some(args) = params.get("launch_args").and_then(|v| v.as_array()) {
                    for arg in args {
                        if let Some(arg_str) = arg.as_str() {
                            cmd.arg(arg_str);
                        }
                    }
                }

                // Add environment variables if provided
                if let Some(env_obj) = params.get("env").and_then(|v| v.as_object()) {
                    for (key, value) in env_obj {
                        if let Some(val_str) = value.as_str() {
                            cmd.env(key, val_str);
                        }
                    }
                }

                let output = cmd
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to launch app: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "action": "launch",
                    "bundle_id": bundle_id,
                    "device_id": device_id,
                    "error": if !output.status.success() {
                        Some(String::from_utf8_lossy(&output.stderr).trim().to_string())
                    } else {
                        None
                    }
                }))
            }

            "terminate" => {
                let bundle_id = params
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing bundle_id for terminate".to_string()))?;

                let output = Command::new("xcrun")
                    .args(["simctl", "terminate", &device_id, bundle_id])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to terminate app: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "action": "terminate",
                    "bundle_id": bundle_id,
                    "device_id": device_id,
                    "error": if !output.status.success() {
                        Some(String::from_utf8_lossy(&output.stderr).trim().to_string())
                    } else {
                        None
                    }
                }))
            }

            "install" => {
                let app_path = params
                    .get("app_path")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing app_path for install".to_string()))?;

                let output = Command::new("xcrun")
                    .args(["simctl", "install", &device_id, app_path])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to install app: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "action": "install",
                    "app_path": app_path,
                    "device_id": device_id,
                    "error": if !output.status.success() {
                        Some(String::from_utf8_lossy(&output.stderr).trim().to_string())
                    } else {
                        None
                    }
                }))
            }

            "uninstall" => {
                let bundle_id = params
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing bundle_id for uninstall".to_string()))?;

                let output = Command::new("xcrun")
                    .args(["simctl", "uninstall", &device_id, bundle_id])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to uninstall app: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "action": "uninstall",
                    "bundle_id": bundle_id,
                    "device_id": device_id,
                    "error": if !output.status.success() {
                        Some(String::from_utf8_lossy(&output.stderr).trim().to_string())
                    } else {
                        None
                    }
                }))
            }

            "list" => {
                let output = Command::new("xcrun")
                    .args(["simctl", "listapps", &device_id])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to list apps: {}", e)))?;

                if output.status.success() {
                    // Parse the plist output
                    let apps_str = String::from_utf8_lossy(&output.stdout);
                    // For now, return raw output - could parse plist in the future
                    Ok(json!({
                        "success": true,
                        "action": "list",
                        "device_id": device_id,
                        "apps": apps_str.to_string(),
                        "note": "Raw plist output - parse for structured data"
                    }))
                } else {
                    Ok(json!({
                        "success": false,
                        "error": String::from_utf8_lossy(&output.stderr).trim().to_string()
                    }))
                }
            }

            "info" => {
                let bundle_id = params
                    .get("bundle_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing bundle_id for info".to_string()))?;

                let output = Command::new("xcrun")
                    .args(["simctl", "get_app_container", &device_id, bundle_id])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to get app info: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "action": "info",
                    "bundle_id": bundle_id,
                    "device_id": device_id,
                    "container_path": if output.status.success() {
                        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
                    } else {
                        None
                    },
                    "error": if !output.status.success() {
                        Some(String::from_utf8_lossy(&output.stderr).trim().to_string())
                    } else {
                        None
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
