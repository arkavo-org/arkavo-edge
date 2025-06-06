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

        let _device_id = match self.get_device_id(&params) {
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

                // NOTE: simctl io does NOT support tap commands
                // Tap coordinates for reference:
                // Cancel button typically at: (196.5, 550.0) for center bottom
                // Alternative locations: (196.5, 500.0), (100.0, 550.0), (293.0, 550.0)

                // Send ESC key via AppleScript
                let esc_script = r#"
                    tell application "Simulator"
                        activate
                        tell application "System Events"
                            key code 53 -- ESC key
                        end tell
                    end tell
                "#;

                let result = Command::new("osascript")
                    .args(["-e", esc_script])
                    .output()
                    .map(|output| output.status.success())
                    .unwrap_or(false);

                Ok(serde_json::json!({
                    "success": result,
                    "action": "dismiss_enrollment_warning",
                    "message": "Attempted to dismiss passkey enrollment warning using ESC key",
                    "note": "For tap interactions, use ui_interaction tool. Cancel button typically at (196.5, 550.0)"
                }))
            }
            "accept_enrollment" => {
                // If there's a button to proceed with enrollment
                // This would typically be in the center of the dialog

                // NOTE: simctl io does NOT support tap commands
                // This functionality requires XCTest bridge or AppleScript
                return Err(TestError::Mcp(
                    "Accept enrollment requires UI interaction. Use ui_interaction tool with tap action instead. Suggested coordinates: (196.5, 450.0) for center of dialog".to_string()
                ));
            }
            "cancel_dialog" => {
                // Generic cancel for any passkey-related dialog

                // NOTE: simctl io does NOT support sendkey command
                // Method 1: Try using AppleScript to send ESC key
                let esc_script = r#"
                    tell application "Simulator"
                        activate
                        tell application "System Events"
                            key code 53 -- ESC key
                        end tell
                    end tell
                "#;

                let esc_output = Command::new("osascript")
                    .args(["-e", esc_script])
                    .output()
                    .map_err(|e| {
                        TestError::Mcp(format!("Failed to send ESC via AppleScript: {}", e))
                    })?;

                // NOTE: simctl io does NOT support tap commands
                // We can only use XCTest bridge or AppleScript for UI interactions
                // Method 2 & 3 removed as they use invalid simctl commands

                Ok(serde_json::json!({
                    "success": esc_output.status.success(),
                    "action": "cancel_dialog",
                    "message": "Attempted to cancel passkey dialog",
                    "methods": ["escape_key"],
                    "note": "Tap commands not available via simctl. Use XCTest bridge or ui_interaction tool instead."
                }))
            }
            "tap_settings" => {
                // If the dialog has a "Settings" button to go to biometric settings
                // This is typically on the right side of the dialog

                // NOTE: simctl io does NOT support tap commands
                // This functionality requires XCTest bridge or AppleScript
                return Err(TestError::Mcp(
                    "Tap settings requires UI interaction. Use ui_interaction tool with tap action instead. Suggested coordinates: (293.0, 450.0) for right side of dialog".to_string()
                ));
            }
            _ => Err(TestError::Mcp(format!("Unsupported action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
