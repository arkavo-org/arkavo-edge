use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;

pub struct BiometricDialogHandler {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl BiometricDialogHandler {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "biometric_dialog_handler".to_string(),
                description: "Handle biometric authentication dialogs without external tools"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["dismiss", "accept", "cancel", "use_passcode"],
                            "description": "How to handle the biometric dialog"
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

    fn send_key_event(&self, _device_id: &str, keycode: &str) -> Result<()> {
        // simctl io sendkey is NOT a valid command
        // Use AppleScript instead
        let key_code = match keycode {
            "escape" => "53",
            "return" => "36",
            "home" => "115", // Home key
            _ => return Err(TestError::Mcp(format!("Unknown keycode: {}", keycode))),
        };
        
        let script = format!(
            r#"tell application "Simulator"
                activate
            end tell
            tell application "System Events"
                key code {}
            end tell"#,
            key_code
        );
        
        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to send key event: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to send key event: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    fn simulate_home_button(&self, device_id: &str) -> Result<()> {
        // Send home button press
        self.send_key_event(device_id, "home")?;
        Ok(())
    }

    fn simulate_cancel_button(&self, device_id: &str) -> Result<()> {
        // ESC key often dismisses dialogs
        self.send_key_event(device_id, "escape")?;
        Ok(())
    }

    fn simulate_passcode_entry(&self, _device_id: &str, passcode: &str) -> Result<()> {
        // simctl io type is NOT a valid command
        // Use AppleScript to type the passcode
        let script = format!(
            r#"tell application "Simulator"
                activate
            end tell
            tell application "System Events"
                tell process "Simulator"
                    set frontmost to true
                    keystroke "{}"
                end tell
            end tell"#,
            passcode
        );
        
        Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to type passcode: {}", e)))?;
            
        Ok(())
    }
}

#[async_trait]
impl Tool for BiometricDialogHandler {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        // Get device ID
        let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            // Verify device exists
            if self.device_manager.get_device(id).is_none() {
                return Ok(json!({
                    "error": {
                        "code": "DEVICE_NOT_FOUND",
                        "message": format!("Device '{}' not found", id),
                        "details": {
                            "suggestion": "Use device_management tool with 'list' action to see available devices"
                        }
                    }
                }));
            }
            id.to_string()
        } else {
            // Use active device
            match self.device_manager.get_active_device() {
                Some(device) => device.id,
                None => {
                    return Ok(json!({
                        "error": {
                            "code": "NO_ACTIVE_DEVICE",
                            "message": "No active device set and no device_id specified",
                            "details": {
                                "suggestion": "Use device_management tool to set an active device or specify device_id"
                            }
                        }
                    }));
                }
            }
        };

        match action {
            "dismiss" => {
                // Try multiple methods to dismiss the dialog
                // First try ESC key
                if self.simulate_cancel_button(&device_id).is_err() {
                    // If that fails, try home button
                    self.simulate_home_button(&device_id).ok();
                }

                Ok(json!({
                    "success": true,
                    "action": "dismiss",
                    "method": "keyboard_events",
                    "device_id": device_id
                }))
            }
            "cancel" => {
                // Specifically try to cancel the dialog
                match self.simulate_cancel_button(&device_id) {
                    Ok(_) => Ok(json!({
                        "success": true,
                        "action": "cancel",
                        "method": "escape_key",
                        "device_id": device_id
                    })),
                    Err(e) => Ok(json!({
                        "error": {
                            "code": "CANCEL_FAILED",
                            "message": e.to_string(),
                            "suggestion": "Try using the biometric_auth tool with 'cancel' action"
                        }
                    })),
                }
            }
            "accept" => {
                // Biometric acceptance cannot be automated
                Ok(json!({
                    "error": {
                        "code": "BIOMETRIC_ACCEPT_NOT_AUTOMATED",
                        "message": "Cannot programmatically accept biometric authentication",
                        "device_id": device_id,
                        "details": {
                            "reason": "iOS Simulator requires manual menu interaction for biometric simulation",
                            "manual_steps": [
                                "1. When biometric dialog is visible",
                                "2. Go to Device > Face ID/Touch ID > Matching Face/Touch"
                            ]
                        }
                    }
                }))
            }
            "use_passcode" => {
                // Default passcode for simulators is often "1234" or "123456"
                let passcode = params
                    .get("passcode")
                    .and_then(|v| v.as_str())
                    .unwrap_or("1234");

                match self.simulate_passcode_entry(&device_id, passcode) {
                    Ok(_) => {
                        // After typing passcode, send return key
                        self.send_key_event(&device_id, "return").ok();

                        Ok(json!({
                            "success": true,
                            "action": "use_passcode",
                            "passcode_length": passcode.len(),
                            "device_id": device_id
                        }))
                    }
                    Err(e) => Ok(json!({
                        "error": {
                            "code": "PASSCODE_ENTRY_FAILED",
                            "message": e.to_string()
                        }
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

// Alternative approach using accessibility features
pub struct AccessibilityDialogHandler {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl AccessibilityDialogHandler {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "accessibility_dialog_handler".to_string(),
                description: "Handle dialogs using accessibility features".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "dialog_type": {
                            "type": "string",
                            "enum": ["biometric", "permission", "alert"],
                            "description": "Type of dialog to handle"
                        },
                        "action": {
                            "type": "string",
                            "enum": ["find_buttons", "press_cancel", "press_ok", "press_allow"],
                            "description": "Action to perform"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID"
                        }
                    },
                    "required": ["dialog_type", "action"]
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for AccessibilityDialogHandler {
    async fn execute(&self, params: Value) -> Result<Value> {
        let dialog_type = params
            .get("dialog_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing dialog_type parameter".to_string()))?;

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        // Get device ID
        let _device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else {
            match self.device_manager.get_active_device() {
                Some(device) => device.id,
                None => {
                    return Ok(json!({
                        "error": {
                            "code": "NO_ACTIVE_DEVICE",
                            "message": "No active device set"
                        }
                    }));
                }
            }
        };

        match (dialog_type, action) {
            ("biometric", "press_cancel") => {
                // Use AppleScript to send ESC key
                let script = r#"tell application "Simulator"
                    activate
                end tell
                tell application "System Events"
                    key code 53 -- ESC key
                end tell"#;
                
                Command::new("osascript")
                    .arg("-e")
                    .arg(script)
                    .output()
                    .ok();

                Ok(json!({
                    "success": true,
                    "dialog_type": dialog_type,
                    "action": action,
                    "method": "keyboard_shortcut"
                }))
            }
            ("biometric", "find_buttons") => {
                // Return known button positions for biometric dialogs
                Ok(json!({
                    "success": true,
                    "dialog_type": dialog_type,
                    "buttons": [
                        {
                            "label": "Cancel",
                            "estimated_position": {
                                "x": 196,
                                "y": 500,
                                "note": "Typically at bottom center of dialog"
                            }
                        },
                        {
                            "label": "Enter Password",
                            "estimated_position": {
                                "x": 196,
                                "y": 450,
                                "note": "Usually above cancel button"
                            }
                        }
                    ],
                    "suggestion": "Use ui_interaction tool with these coordinates"
                }))
            }
            _ => Ok(json!({
                "success": false,
                "error": format!("Unsupported combination: {} + {}", dialog_type, action)
            })),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
