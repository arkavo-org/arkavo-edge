use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct UrlDialogHandler {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl UrlDialogHandler {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "url_dialog".to_string(),
                description: "Handle iOS system URL scheme confirmation dialogs that appear when opening deep links".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["tap_open", "tap_cancel", "auto_accept", "detect"],
                            "description": "Action to perform on URL dialog"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "wait_timeout": {
                            "type": "integer",
                            "description": "Timeout in seconds to wait for dialog (default: 2)"
                        }
                    },
                    "required": ["action"]
                }),
            },
            device_manager,
        }
    }

    fn get_device_id(&self, params: &Value) -> Result<String> {
        if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            Ok(id.to_string())
        } else {
            match self.device_manager.get_active_device() {
                Some(device) => Ok(device.id),
                None => {
                    self.device_manager.refresh_devices().ok();
                    match self.device_manager.get_booted_devices().first() {
                        Some(device) => Ok(device.id.clone()),
                        None => Err(TestError::Mcp("No booted iOS device found".to_string())),
                    }
                }
            }
        }
    }

    fn get_open_button_coordinates(&self, device_name: &str) -> (f64, f64) {
        // These are the typical coordinates for the "Open" button in URL confirmation dialogs
        // Based on different iPhone models
        match device_name {
            name if name.contains("iPhone 16 Pro Max") => (312.0, 515.0),
            name if name.contains("iPhone 16 Pro") => (195.0, 490.0), // Centered "Open" button
            name if name.contains("iPhone 16") => (195.0, 490.0),
            name if name.contains("iPhone 15 Pro Max") => (312.0, 515.0),
            name if name.contains("iPhone 15 Pro") => (195.0, 490.0),
            name if name.contains("iPhone 15") => (195.0, 490.0),
            name if name.contains("iPhone 14 Pro Max") => (312.0, 515.0),
            name if name.contains("iPhone 14 Pro") => (195.0, 490.0),
            name if name.contains("iPhone 14") => (195.0, 490.0),
            name if name.contains("iPhone 13") => (195.0, 475.0),
            name if name.contains("iPhone 12") => (195.0, 475.0),
            name if name.contains("iPhone SE") => (160.0, 420.0),
            name if name.contains("iPad") => (512.0, 600.0),
            _ => (195.0, 490.0), // Default for unknown devices
        }
    }

    async fn tap_open(&self, device_id: &str) -> Result<Value> {
        let device = self
            .device_manager
            .get_device(device_id)
            .ok_or_else(|| TestError::Mcp(format!("Device {} not found", device_id)))?;

        let (x, y) = self.get_open_button_coordinates(&device.name);

        eprintln!(
            "[UrlDialogHandler] Tapping 'Open' button at ({}, {}) for {}",
            x, y, device.name
        );

        // Use idb to tap the button
        let output = Command::new("idb")
            .args([
                "ui",
                "tap",
                &x.to_string(),
                &y.to_string(),
                "--udid",
                device_id,
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute idb tap: {}", e)))?;

        if !output.status.success() {
            let _stderr = String::from_utf8_lossy(&output.stderr);

            // If idb fails, try using simctl
            eprintln!("[UrlDialogHandler] idb failed, trying simctl approach...");

            let simctl_output = Command::new("xcrun")
                .args([
                    "simctl",
                    "io",
                    device_id,
                    "tap",
                    &x.to_string(),
                    &y.to_string(),
                ])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to execute simctl tap: {}", e)))?;

            if !simctl_output.status.success() {
                return Ok(json!({
                    "success": false,
                    "error": "Failed to tap Open button",
                    "details": String::from_utf8_lossy(&simctl_output.stderr).to_string(),
                    "coordinates": {"x": x, "y": y},
                    "device": device.name
                }));
            }
        }

        Ok(json!({
            "success": true,
            "action": "tap_open",
            "message": "Successfully tapped 'Open' button",
            "coordinates": {"x": x, "y": y},
            "device": device.name
        }))
    }
}

#[async_trait]
impl Tool for UrlDialogHandler {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        let device_id = self.get_device_id(&params)?;

        match action {
            "tap_open" => self.tap_open(&device_id).await,

            "tap_cancel" => {
                // Cancel button is typically on the left side
                let device = self
                    .device_manager
                    .get_device(&device_id)
                    .ok_or_else(|| TestError::Mcp(format!("Device {} not found", device_id)))?;

                let (x, y) = match device.name.as_str() {
                    name if name.contains("iPhone 16 Pro") => (78.0, 490.0),
                    name if name.contains("iPhone 14") => (78.0, 490.0),
                    _ => (78.0, 490.0),
                };

                let output = Command::new("idb")
                    .args([
                        "ui",
                        "tap",
                        &x.to_string(),
                        &y.to_string(),
                        "--udid",
                        &device_id,
                    ])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to execute tap: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "action": "tap_cancel",
                    "message": if output.status.success() {
                        "Successfully tapped 'Cancel' button"
                    } else {
                        "Failed to tap Cancel button"
                    },
                    "coordinates": {"x": x, "y": y}
                }))
            }

            "auto_accept" => {
                // Wait briefly for dialog to appear
                let wait_timeout = params
                    .get("wait_timeout")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(2);

                eprintln!(
                    "[UrlDialogHandler] Waiting {}s for URL dialog to appear...",
                    wait_timeout
                );
                thread::sleep(Duration::from_secs(wait_timeout));

                // Tap the Open button
                self.tap_open(&device_id).await
            }

            "detect" => {
                // Take a screenshot and check for dialog presence
                // This is a simplified detection - in production you'd use image recognition
                Ok(json!({
                    "success": true,
                    "action": "detect",
                    "message": "Dialog detection not fully implemented",
                    "hint": "Use 'auto_accept' after opening a URL to automatically handle the dialog"
                }))
            }

            _ => Err(TestError::Mcp(format!("Unknown action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
