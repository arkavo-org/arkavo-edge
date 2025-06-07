use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;

pub struct FaceIdController {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl FaceIdController {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "face_id_control".to_string(),
                description: "Control Face ID enrollment and matching state in iOS Simulator"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["enroll", "unenroll", "match", "no_match"],
                            "description": "Face ID control action (matches simulator menu options)"
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
}

#[async_trait]
impl Tool for FaceIdController {
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
            "enroll" => {
                // Try AppleScript automation
                let applescript = r#"
                    tell application "Simulator"
                        activate
                        delay 0.5
                        tell application "System Events"
                            tell process "Simulator"
                                -- Click to ensure menu item is checked
                                set enrollMenuItem to menu item "Enrolled" of menu "Face ID" of menu item "Face ID" of menu "Device" of menu bar 1
                                if value of attribute "AXMenuItemMarkChar" of enrollMenuItem is not "✓" then
                                    click enrollMenuItem
                                end if
                            end tell
                        end tell
                    end tell
                "#;

                let result = Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output();

                match result {
                    Ok(output) if output.status.success() => Ok(json!({
                        "success": true,
                        "action": "enroll",
                        "device_id": device_id,
                        "message": "Face ID enrollment enabled via AppleScript",
                        "method": "applescript",
                        "note": "App may need restart for enrollment to take effect"
                    })),
                    _ => Ok(json!({
                        "success": false,
                        "action": "enroll",
                        "device_id": device_id,
                        "error": {
                            "code": "FACE_ID_ENROLLMENT_FAILED",
                            "message": "Unable to automate Face ID enrollment",
                            "details": {
                                "attempted_method": "AppleScript automation",
                                "possible_reasons": [
                                    "Accessibility permissions not granted",
                                    "Simulator UI has changed",
                                    "Running in headless/CI environment"
                                ],
                                "manual_steps": [
                                    "1. Focus on the iOS Simulator window",
                                    "2. In menu bar: Features > Face ID > Enrolled (check it)",
                                    "3. Enrollment persists until manually unchecked"
                                ],
                                "grant_permissions": "System Preferences > Security & Privacy > Privacy > Accessibility > Add Terminal/IDE"
                            }
                        }
                    })),
                }
            }
            "unenroll" => {
                // Face ID unenrollment cannot be done programmatically
                Ok(json!({
                    "success": false,
                    "action": "unenroll",
                    "device_id": device_id,
                    "error": {
                        "code": "FACE_ID_UNENROLL_NOT_AUTOMATED",
                        "message": "Face ID unenrollment requires manual interaction",
                        "details": {
                            "manual_steps": [
                                "1. In menu bar: Features > Face ID > Enrolled (uncheck it)"
                            ]
                        }
                    }
                }))
            }
            "match" => {
                // Try AppleScript automation
                let applescript = r#"
                    tell application "Simulator"
                        activate
                        tell application "System Events"
                            tell process "Simulator"
                                click menu item "Matching Face" of menu "Face ID" of menu item "Face ID" of menu "Device" of menu bar 1
                            end tell
                        end tell
                    end tell
                "#;

                let result = Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output();

                match result {
                    Ok(output) if output.status.success() => Ok(json!({
                        "success": true,
                        "action": "match",
                        "device_id": device_id,
                        "message": "Face ID match triggered via AppleScript",
                        "method": "applescript",
                        "timing_critical": "Must be executed while biometric prompt is visible"
                    })),
                    _ => Ok(json!({
                        "success": false,
                        "action": "match",
                        "device_id": device_id,
                        "error": {
                            "code": "FACE_ID_MATCH_FAILED",
                            "message": "Unable to trigger Face ID match",
                            "details": {
                                "manual_steps": [
                                    "1. Ensure Face ID is enrolled",
                                    "2. When biometric prompt appears",
                                    "3. Go to Features > Face ID > Matching Face"
                                ],
                                "timing": "Must be done while biometric prompt is active"
                            }
                        }
                    })),
                }
            }
            "no_match" => {
                // Try AppleScript automation
                let applescript = r#"
                    tell application "Simulator"
                        activate
                        tell application "System Events"
                            tell process "Simulator"
                                click menu item "Non-matching Face" of menu "Face ID" of menu item "Face ID" of menu "Device" of menu bar 1
                            end tell
                        end tell
                    end tell
                "#;

                let result = Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output();

                match result {
                    Ok(output) if output.status.success() => Ok(json!({
                        "success": true,
                        "action": "no_match",
                        "device_id": device_id,
                        "message": "Face ID non-match triggered via AppleScript",
                        "method": "applescript"
                    })),
                    _ => Ok(json!({
                        "success": false,
                        "action": "no_match",
                        "device_id": device_id,
                        "error": {
                            "code": "FACE_ID_NOMATCH_FAILED",
                            "message": "Unable to trigger Face ID non-match",
                            "details": {
                                "manual_steps": [
                                    "1. When biometric prompt appears",
                                    "2. Go to Features > Face ID > Non-matching Face"
                                ]
                            }
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

// Helper to check current Face ID enrollment status
pub struct FaceIdStatusChecker {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl FaceIdStatusChecker {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "face_id_status".to_string(),
                description: "Check current Face ID enrollment and configuration status"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        }
                    }
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for FaceIdStatusChecker {
    async fn execute(&self, params: Value) -> Result<Value> {
        // Get device ID
        let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
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

        // Check if device supports biometric
        let biometric_check = Command::new("xcrun")
            .args([
                "simctl",
                "spawn",
                &device_id,
                "notifyutil",
                "-g",
                "com.apple.BiometricKit.enrollmentChanged",
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to check Face ID status: {}", e)))?;

        let status_output = String::from_utf8_lossy(&biometric_check.stdout);
        let is_enrolled = status_output.contains("1");

        Ok(json!({
            "success": true,
            "device_id": device_id,
            "face_id_enrolled": is_enrolled,
            "raw_status": status_output.trim(),
            "available_actions": {
                "enroll": "Enable Face ID (Features > Face ID > Enrolled)",
                "unenroll": "Disable Face ID (clear enrollment)",
                "match": "Simulate successful Face ID scan (Features > Face ID > Matching Face)",
                "no_match": "Simulate failed Face ID scan (Features > Face ID > Non-matching Face)"
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
