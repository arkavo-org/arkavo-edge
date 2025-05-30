use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::process::Command;
use std::sync::Arc;

pub struct PasskeyDialogHandler {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl PasskeyDialogHandler {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "passkey_dialog".to_string(),
                description: "Handle iOS passkey/biometric enrollment dialogs. IMPORTANT: If you see 'Simulator requires enrolled biometrics to use passkeys' dialog, this is NOT a regular system dialog - use THIS tool with 'dismiss_enrollment_warning' action. The dialog appears when trying to use passkeys without enrolled biometrics. To proceed with sign-in: 1) Use this tool to dismiss the warning, 2) The app may fallback to password login, OR 3) First enroll biometrics using biometric_auth with 'enroll' action, then retry.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["dismiss_enrollment_warning", "accept_enrollment", "cancel_dialog", "tap_settings"],
                            "description": "Action to perform on passkey dialog"
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
}

#[async_trait]
impl Tool for PasskeyDialogHandler {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        let device_id = match self.get_device_id(&params) {
            Ok(id) => id,
            Err(e) => {
                return Ok(serde_json::json!({
                    "error": {
                        "code": "DEVICE_ERROR",
                        "message": e.to_string()
                    }
                }));
            }
        };

        match action {
            "dismiss_enrollment_warning" => {
                // For dialogs that say "Simulator requires enrolled biometrics to use passkeys"
                // These typically have a "Cancel" button at the bottom
                
                // Method 1: Try to tap common Cancel button location
                let cancel_locations = vec![
                    (196.5, 550.0),  // Center bottom for iPhone Pro
                    (196.5, 500.0),  // Slightly higher
                    (196.5, 600.0),  // Slightly lower
                    (100.0, 550.0),  // Left side
                    (293.0, 550.0),  // Right side
                ];

                let mut success = false;
                for (x, y) in &cancel_locations {
                    let output = Command::new("xcrun")
                        .args(["simctl", "io", &device_id, "tap", &x.to_string(), &y.to_string()])
                        .output()
                        .map_err(|e| TestError::Mcp(format!("Failed to tap: {}", e)))?;
                    
                    if output.status.success() {
                        success = true;
                        break;
                    }
                }

                // Method 2: Send ESC key as backup
                Command::new("xcrun")
                    .args(["simctl", "io", &device_id, "sendkey", "escape"])
                    .output()
                    .ok();

                Ok(serde_json::json!({
                    "success": success,
                    "action": "dismiss_enrollment_warning",
                    "message": "Attempted to dismiss passkey enrollment warning",
                    "tapped_locations": cancel_locations,
                    "note": "Also sent ESC key as backup"
                }))
            }
            "accept_enrollment" => {
                // If there's a button to proceed with enrollment
                // This would typically be in the center of the dialog
                
                let ok_locations = vec![
                    (196.5, 450.0),  // Center middle
                    (196.5, 400.0),  // Higher center
                    (196.5, 500.0),  // Lower center
                ];

                let mut success = false;
                for (x, y) in &ok_locations {
                    let output = Command::new("xcrun")
                        .args(["simctl", "io", &device_id, "tap", &x.to_string(), &y.to_string()])
                        .output()
                        .map_err(|e| TestError::Mcp(format!("Failed to tap: {}", e)))?;
                    
                    if output.status.success() {
                        success = true;
                        break;
                    }
                }

                Ok(serde_json::json!({
                    "success": success,
                    "action": "accept_enrollment",
                    "message": "Attempted to accept passkey enrollment",
                    "tapped_locations": ok_locations
                }))
            }
            "cancel_dialog" => {
                // Generic cancel for any passkey-related dialog
                
                // Method 1: Send ESC key
                let esc_output = Command::new("xcrun")
                    .args(["simctl", "io", &device_id, "sendkey", "escape"])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to send ESC: {}", e)))?;

                // Method 2: Try tapping outside the dialog to dismiss
                Command::new("xcrun")
                    .args(["simctl", "io", &device_id, "tap", "50", "100"])
                    .output()
                    .ok();

                // Method 3: Try common Cancel button locations
                Command::new("xcrun")
                    .args(["simctl", "io", &device_id, "tap", "196", "550"])
                    .output()
                    .ok();

                Ok(serde_json::json!({
                    "success": esc_output.status.success(),
                    "action": "cancel_dialog",
                    "message": "Attempted to cancel passkey dialog",
                    "methods": ["escape_key", "tap_outside", "tap_cancel_button"]
                }))
            }
            "tap_settings" => {
                // If the dialog has a "Settings" button to go to biometric settings
                // This is typically on the right side of the dialog
                
                let settings_locations = vec![
                    (293.0, 450.0),  // Right middle
                    (300.0, 500.0),  // Right lower
                    (280.0, 400.0),  // Right upper
                ];

                let mut success = false;
                for (x, y) in &settings_locations {
                    let output = Command::new("xcrun")
                        .args(["simctl", "io", &device_id, "tap", &x.to_string(), &y.to_string()])
                        .output()
                        .map_err(|e| TestError::Mcp(format!("Failed to tap: {}", e)))?;
                    
                    if output.status.success() {
                        success = true;
                        break;
                    }
                }

                Ok(serde_json::json!({
                    "success": success,
                    "action": "tap_settings",
                    "message": "Attempted to tap Settings button",
                    "tapped_locations": settings_locations,
                    "note": "This may open the Settings app"
                }))
            }
            _ => Err(TestError::Mcp(format!("Unsupported action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}