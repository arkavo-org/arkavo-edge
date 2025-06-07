use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use std::process::Command;
use std::thread;
use std::time::Duration;

pub struct EnrollmentDialogHandler {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl EnrollmentDialogHandler {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "enrollment_dialog".to_string(),
                description: "Handle the 'Simulator requires enrolled biometrics to use passkeys' dialog with device-specific coordinates".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["get_cancel_coordinates", "tap_cancel", "dismiss", "handle_automatically", "wait_for_dialog"],
                            "description": "Action to perform on enrollment dialog"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
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
            if self.device_manager.get_device(id).is_none() {
                return Err(TestError::Mcp(format!("Device '{}' not found", id)));
            }
            Ok(id.to_string())
        } else {
            match self.device_manager.get_active_device() {
                Some(device) => Ok(device.id),
                None => {
                    self.device_manager.refresh_devices().ok();
                    match self.device_manager.get_booted_devices().first() {
                        Some(device) => Ok(device.id.clone()),
                        None => Err(TestError::Mcp("No booted device found".to_string())),
                    }
                }
            }
        }
    }

    fn dismiss_dialog_with_keyboard(&self) -> Result<()> {
        // Use AppleScript to send ESC key to dismiss the dialog
        let script = r#"tell application "Simulator"
            activate
        end tell
        tell application "System Events"
            key code 53 -- ESC key
        end tell"#;

        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute AppleScript: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to send ESC key: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    fn get_cancel_coordinates(&self, device_type: &str) -> (f64, f64) {
        // Device-specific coordinates for the Cancel button
        // Based on actual observations from screenshots
        match device_type {
            s if s.contains("iPhone-16-Pro-Max") || s.contains("iPhone 16 Pro Max") => {
                // iPhone 16 Pro Max: 430x932 logical resolution
                // Cancel button is centered horizontally at bottom of dialog
                (215.0, 830.0)
            }
            s if s.contains("iPhone-16-Pro") || s.contains("iPhone 16 Pro") => {
                // iPhone 16 Pro: 393x852 logical resolution
                (196.5, 750.0)
            }
            s if s.contains("iPhone-16-Plus") || s.contains("iPhone 16 Plus") => {
                // iPhone 16 Plus: 428x926 logical resolution
                (214.0, 820.0)
            }
            s if s.contains("iPhone-16") || s.contains("iPhone 16") => {
                // iPhone 16: 390x844 logical resolution
                (195.0, 740.0)
            }
            s if s.contains("iPhone-15") => {
                // iPhone 15 variants have similar layouts
                if s.contains("Pro-Max") {
                    (215.0, 830.0)
                } else if s.contains("Pro") {
                    (196.5, 750.0)
                } else if s.contains("Plus") {
                    (214.0, 820.0)
                } else {
                    (195.0, 740.0)
                }
            }
            s if s.contains("iPhone-SE") => {
                // iPhone SE: 375x667 logical resolution
                (187.5, 550.0)
            }
            s if s.contains("iPad") => {
                // iPad: 820x1180 logical resolution (typical)
                (410.0, 900.0)
            }
            _ => {
                // Default to iPhone Pro size
                (196.5, 750.0)
            }
        }
    }
}

#[async_trait]
impl Tool for EnrollmentDialogHandler {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        let device_id = match self.get_device_id(&params) {
            Ok(id) => id,
            Err(e) => {
                return Ok(json!({
                    "error": {
                        "code": "DEVICE_ERROR",
                        "message": e.to_string()
                    }
                }));
            }
        };

        // Get device info
        let device_info = self.device_manager.get_device(&device_id);
        let device_type = device_info
            .as_ref()
            .map(|d| d.device_type.as_str())
            .unwrap_or("unknown");

        match action {
            "get_cancel_coordinates" => {
                let (x, y) = self.get_cancel_coordinates(device_type);

                Ok(json!({
                    "success": true,
                    "action": "get_cancel_coordinates",
                    "device_type": device_type,
                    "cancel_button": {
                        "x": x,
                        "y": y,
                        "description": "Center of Cancel button in enrollment dialog"
                    },
                    "usage": {
                        "description": "Use these coordinates with ui_interaction tool",
                        "example": {
                            "tool": "ui_interaction",
                            "params": {
                                "action": "tap",
                                "target": {
                                    "x": x,
                                    "y": y
                                }
                            }
                        }
                    }
                }))
            }
            "tap_cancel" => {
                let (x, y) = self.get_cancel_coordinates(device_type);

                Ok(json!({
                    "success": true,
                    "action": "tap_cancel",
                    "message": "To tap the Cancel button, use ui_interaction tool with the provided coordinates",
                    "coordinates": {
                        "x": x,
                        "y": y
                    },
                    "next_step": {
                        "tool": "ui_interaction",
                        "params": {
                            "action": "tap",
                            "target": {
                                "x": x,
                                "y": y
                            }
                        }
                    }
                }))
            }
            "dismiss" => {
                // Try to dismiss the dialog using keyboard shortcuts
                self.dismiss_dialog_with_keyboard()?;
                
                Ok(json!({
                    "success": true,
                    "action": "dismiss",
                    "method": "keyboard_shortcut",
                    "device_id": device_id,
                    "message": "Attempted to dismiss enrollment dialog using ESC key"
                }))
            }
            "handle_automatically" => {
                // First try keyboard dismissal
                if self.dismiss_dialog_with_keyboard().is_ok() {
                    return Ok(json!({
                        "success": true,
                        "action": "handle_automatically",
                        "method": "keyboard_dismissal",
                        "device_id": device_id
                    }));
                }
                
                // If that fails, provide coordinates for manual tap
                let (x, y) = self.get_cancel_coordinates(device_type);
                Ok(json!({
                    "success": false,
                    "action": "handle_automatically",
                    "fallback": {
                        "method": "manual_tap",
                        "coordinates": {
                            "x": x,
                            "y": y
                        },
                        "next_step": {
                            "tool": "ui_interaction",
                            "params": {
                                "action": "tap",
                                "target": {
                                    "x": x,
                                    "y": y
                                }
                            }
                        }
                    }
                }))
            }
            "wait_for_dialog" => {
                // Wait a moment for dialog to appear
                thread::sleep(Duration::from_millis(500));
                
                Ok(json!({
                    "success": true,
                    "action": "wait_for_dialog",
                    "device_id": device_id,
                    "message": "Waited for enrollment dialog to appear",
                    "next_actions": [
                        "Use 'dismiss' or 'handle_automatically' to handle the dialog",
                        "Use 'get_cancel_coordinates' to get button coordinates"
                    ]
                }))
            }
            _ => Err(TestError::Mcp(format!("Unsupported action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
