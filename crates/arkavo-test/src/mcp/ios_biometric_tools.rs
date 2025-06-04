use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::process::Command;
use std::sync::Arc;

pub struct BiometricKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl BiometricKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "biometric_auth".to_string(),
                description: "Handle Face ID/Touch ID authentication for both simulators and devices. For 'enroll' on simulator: returns instructions for manual enrollment (Device > Face ID > Enrolled) as this often cannot be done programmatically. After manual enrollment, use passkey_dialog tool to dismiss any enrollment warning dialogs. For 'match'/'fail'/'cancel': provides multiple fallback methods including simctl commands, notifications, and taps. Works best when Face ID is already enrolled in simulator.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["enroll", "match", "fail", "cancel"],
                            "description": "Biometric action to perform"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "biometric_type": {
                            "type": "string",
                            "enum": ["face_id", "touch_id"],
                            "description": "Type of biometric authentication"
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


    fn try_applescript_biometric(&self, device_id: &str, action: &str, biometric_type: &str) -> bool {
        // Get device name for AppleScript
        let _device = match self.device_manager.get_device(device_id) {
            Some(d) => d,
            None => return false,
        };

        let menu_item = match (biometric_type, action) {
            ("face_id", "match") => "Matching Face",
            ("face_id", "nomatch") => "Non-matching Face", 
            ("touch_id", "match") => "Matching Touch",
            ("touch_id", "nomatch") => "Non-matching Touch",
            _ => return false,
        };

        // AppleScript to click menu item
        let applescript = format!(
            r#"
            tell application "Simulator"
                activate
                tell application "System Events"
                    tell process "Simulator"
                        click menu item "{}" of menu "Face ID" of menu item "Face ID" of menu "Device" of menu bar 1
                    end tell
                end tell
            end tell
            "#,
            menu_item
        );

        // Execute AppleScript
        let result = Command::new("osascript")
            .arg("-e")
            .arg(&applescript)
            .output();

        match result {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    fn try_keyboard_shortcut_biometric(&self, _device_id: &str, action: &str) -> bool {
        // Some biometric dialogs may respond to keyboard shortcuts
        // This is a placeholder for potential keyboard-based workarounds
        
        // For now, we can try common shortcuts like:
        // Cmd+Shift+M for "Match" (hypothetical)
        // Cmd+Shift+N for "No Match" (hypothetical)
        
        // Note: These are not standard shortcuts and likely won't work,
        // but we're trying all possible workarounds before failing
        
        match action {
            "match" => {
                // Try to send a keyboard shortcut (this is speculative)
                let result = Command::new("osascript")
                    .arg("-e")
                    .arg(r#"tell application "Simulator" to activate"#)
                    .output();
                
                if result.is_ok() {
                    // Give focus time to switch
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    
                    // Try sending Enter key which might confirm biometric
                    Command::new("osascript")
                        .arg("-e")
                        .arg(r#"tell application "System Events" to keystroke return"#)
                        .output()
                        .ok();
                }
                false // Keyboard shortcuts for biometric are not standard
            }
            _ => false,
        }
    }
}

#[async_trait]
impl Tool for BiometricKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        let biometric_type = params
            .get("biometric_type")
            .and_then(|v| v.as_str())
            .unwrap_or("face_id");

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
            "enroll" => {
                // Try AppleScript to toggle enrollment
                let applescript = r#"
                    tell application "Simulator"
                        activate
                        tell application "System Events"
                            tell process "Simulator"
                                click menu item "Enrolled" of menu "Face ID" of menu item "Face ID" of menu "Device" of menu bar 1
                            end tell
                        end tell
                    end tell
                "#;

                let result = Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output();

                match result {
                    Ok(output) if output.status.success() => {
                        Ok(serde_json::json!({
                            "success": true,
                            "action": "enroll",
                            "biometric_type": biometric_type,
                            "message": "Toggled biometric enrollment via AppleScript",
                            "method": "applescript",
                            "warning": "This toggles the enrollment state. Check current state with face_id_status tool.",
                            "note": "May require app restart to take effect"
                        }))
                    }
                    _ => {
                        // AppleScript failed, return honest failure
                        Ok(serde_json::json!({
                            "success": false,
                            "action": "enroll",
                            "biometric_type": biometric_type,
                            "error": {
                                "code": "ENROLLMENT_AUTOMATION_FAILED",
                                "message": "Unable to automate biometric enrollment",
                                "details": {
                                    "reason": "AppleScript automation failed - may require accessibility permissions",
                                    "manual_steps": [
                                        "1. In the Simulator menu bar, go to Device > Face ID/Touch ID",
                                        "2. Check the 'Enrolled' option",
                                        "3. Enrollment persists until manually unchecked"
                                    ],
                                    "troubleshooting": [
                                        "Ensure Terminal/IDE has accessibility permissions",
                                        "System Preferences > Security & Privacy > Privacy > Accessibility"
                                    ],
                                    "alternatives": {
                                        "appium": "Use Appium's mobile:enrollBiometric command"
                                    }
                                }
                            }
                        }))
                    }
                }
            }
            "match" => {
                // Method 1: Try AppleScript automation (if available)
                if self.try_applescript_biometric(&device_id, "match", biometric_type) {
                    return Ok(serde_json::json!({
                        "success": true,
                        "action": "match",
                        "biometric_type": biometric_type,
                        "message": "Biometric authentication triggered via AppleScript",
                        "method": "applescript",
                        "warning": "This method may be unreliable in CI/headless environments"
                    }));
                }

                // Method 2: Try keyboard shortcuts (may work in some cases)
                if self.try_keyboard_shortcut_biometric(&device_id, "match") {
                    return Ok(serde_json::json!({
                        "success": true,
                        "action": "match",
                        "biometric_type": biometric_type,
                        "message": "Biometric authentication attempted via keyboard shortcut",
                        "method": "keyboard_shortcut"
                    }));
                }

                // All methods failed - return honest failure
                Ok(serde_json::json!({
                    "success": false,
                    "action": "match",
                    "biometric_type": biometric_type,
                    "error": {
                        "code": "BIOMETRIC_AUTOMATION_FAILED",
                        "message": "Unable to trigger biometric authentication programmatically",
                        "attempted_methods": ["applescript", "keyboard_shortcut"],
                        "details": {
                            "reason": "iOS Simulator biometric control requires manual interaction or specialized tools",
                            "manual_steps": [
                                "1. In Simulator menu, go to Device > Face ID/Touch ID",
                                "2. Ensure 'Enrolled' is checked",
                                "3. Select 'Matching Face' or 'Matching Touch' when biometric prompt appears"
                            ],
                            "alternatives": {
                                "appium": "Use Appium with XCUITest driver and mobile:sendBiometricMatch command",
                                "xcuitest": "Use XCUITest native APIs within test bundles",
                                "cloud_services": "Consider BrowserStack or Perfecto for automated biometric testing"
                            }
                        }
                    }
                }))
            }
            "fail" => {
                // Try AppleScript automation first
                if self.try_applescript_biometric(&device_id, "nomatch", biometric_type) {
                    return Ok(serde_json::json!({
                        "success": true,
                        "action": "fail",
                        "biometric_type": biometric_type,
                        "message": "Biometric failure triggered via AppleScript",
                        "method": "applescript"
                    }));
                }

                // Return honest failure
                Ok(serde_json::json!({
                    "success": false,
                    "action": "fail",
                    "biometric_type": biometric_type,
                    "error": {
                        "code": "BIOMETRIC_FAIL_AUTOMATION_FAILED",
                        "message": "Unable to simulate biometric failure programmatically",
                        "details": {
                            "manual_steps": [
                                "1. When biometric prompt appears",
                                "2. Go to Device > Face ID/Touch ID > Non-matching Face/Touch"
                            ],
                            "alternatives": {
                                "appium": "Use Appium's mobile:sendBiometricMatch with match:false"
                            }
                        }
                    }
                }))
            }
            "cancel" => {
                // Try to cancel via ESC key - this may work for some dialogs
                let esc_result = Command::new("xcrun")
                    .args(["simctl", "io", &device_id, "sendkey", "escape"])
                    .output();

                match esc_result {
                    Ok(output) if output.status.success() => {
                        Ok(serde_json::json!({
                            "success": true,
                            "action": "cancel",
                            "biometric_type": biometric_type,
                            "message": "Sent ESC key to attempt dialog cancellation",
                            "warning": "This may not work for all biometric dialogs. Manual cancellation may be required.",
                            "note": "For 'Simulator requires enrolled biometrics' dialogs, use passkey_dialog tool instead."
                        }))
                    }
                    _ => {
                        Ok(serde_json::json!({
                            "success": false,
                            "action": "cancel",
                            "biometric_type": biometric_type,
                            "error": {
                                "code": "CANCEL_FAILED",
                                "message": "Unable to programmatically cancel biometric dialog",
                                "details": {
                                    "manual_steps": [
                                        "Tap the Cancel button in the biometric dialog",
                                        "Or press ESC key while Simulator has focus"
                                    ]
                                }
                            }
                        }))
                    }
                }
            }
            _ => Err(TestError::Mcp(format!("Unsupported action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct SystemDialogKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl SystemDialogKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "system_dialog".to_string(),
                description: "Handle iOS system dialogs and alerts. NOTE: For 'Simulator requires enrolled biometrics' dialogs, use the passkey_dialog tool instead with 'dismiss_enrollment_warning' action. This tool is for general system alerts like permissions, notifications, etc.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["accept", "dismiss", "allow", "deny"],
                            "description": "Action to perform on system dialog"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "button_text": {
                            "type": "string",
                            "description": "Specific button text to tap"
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
impl Tool for SystemDialogKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        let button_text = params.get("button_text").and_then(|v| v.as_str());

        // Get target device
        let _device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            // Verify device exists
            if self.device_manager.get_device(id).is_none() {
                return Ok(serde_json::json!({
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
                    return Ok(serde_json::json!({
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

        // Map action to common button texts
        let button = match (action, button_text) {
            (_, Some(text)) => text,
            ("accept", _) => "OK",
            ("dismiss", _) => "Cancel",
            ("allow", _) => "Allow",
            ("deny", _) => "Don't Allow",
            _ => "OK",
        };

        // Try AppleScript to click the button
        let applescript = format!(
            r#"
            tell application "Simulator"
                activate
                delay 0.5
                tell application "System Events"
                    tell process "Simulator"
                        tell window 1
                            if exists button "{}" then
                                click button "{}"
                                return "success"
                            else
                                -- Try to find any button containing the text
                                set allButtons to buttons
                                repeat with aButton in allButtons
                                    if name of aButton contains "{}" then
                                        click aButton
                                        return "success"
                                    end if
                                end repeat
                                return "not_found"
                            end if
                        end tell
                    end tell
                end tell
            end tell
            "#,
            button, button, button
        );

        let result = Command::new("osascript")
            .arg("-e")
            .arg(&applescript)
            .output();

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if output.status.success() && stdout == "success" {
                    Ok(serde_json::json!({
                        "success": true,
                        "action": action,
                        "button_tapped": button,
                        "message": format!("Successfully tapped '{}' button", button),
                        "method": "applescript"
                    }))
                } else {
                    // AppleScript failed - return honest failure
                    Ok(serde_json::json!({
                        "success": false,
                        "action": action,
                        "error": {
                            "code": "DIALOG_INTERACTION_FAILED",
                            "message": format!("Unable to find or tap '{}' button", button),
                            "details": {
                                "attempted_button": button,
                                "reason": if stdout == "not_found" {
                                    "Button not found in current dialog"
                                } else {
                                    "AppleScript execution failed"
                                },
                                "suggestions": [
                                    "Ensure a dialog is visible in the Simulator",
                                    "Check if the button text matches exactly",
                                    "Try using ui_interaction tool with specific coordinates",
                                    "For deep link dialogs, the button might be 'Open' instead of 'OK'"
                                ],
                                "manual_fallback": "Manually tap the button in the Simulator"
                            }
                        }
                    }))
                }
            }
            Err(e) => {
                Ok(serde_json::json!({
                    "success": false,
                    "action": action,
                    "error": {
                        "code": "APPLESCRIPT_FAILED",
                        "message": format!("Failed to execute AppleScript: {}", e),
                        "details": {
                            "grant_permissions": "System Preferences > Security & Privacy > Privacy > Accessibility",
                            "alternative": "Use ui_interaction tool with known button coordinates"
                        }
                    }
                }))
            }
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
