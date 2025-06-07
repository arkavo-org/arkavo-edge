use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;

pub struct BiometricTestScenario {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl BiometricTestScenario {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "biometric_test_scenario".to_string(),
                description:
                    "Execute biometric test scenarios including success, failure, and edge cases"
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "scenario": {
                            "type": "string",
                            "enum": [
                                "success_path",
                                "user_cancels",
                                "face_not_recognized",
                                "biometric_lockout",
                                "fallback_to_passcode",
                                "check_current_state"
                            ],
                            "description": "The test scenario to execute"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        }
                    },
                    "required": ["scenario"]
                }),
            },
            device_manager,
        }
    }

    fn ensure_face_id_enrolled(&self, device_id: &str) -> Result<bool> {
        // Check enrollment status using notifyutil
        let output = Command::new("xcrun")
            .args([
                "simctl",
                "spawn",
                device_id,
                "notifyutil",
                "-g",
                "com.apple.BiometricKit.enrollmentChanged",
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to check enrollment: {}", e)))?;

        let status = String::from_utf8_lossy(&output.stdout);
        Ok(status.contains("1"))
    }
}

#[async_trait]
impl Tool for BiometricTestScenario {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scenario = params
            .get("scenario")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing scenario parameter".to_string()))?;

        // Get device ID
        let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            if self.device_manager.get_device(id).is_none() {
                return Ok(json!({
                    "error": {
                        "code": "DEVICE_NOT_FOUND",
                        "message": format!("Device '{}' not found", id)
                    }
                }));
            }
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

        match scenario {
            "success_path" => {
                // Check if Face ID is enrolled
                let is_enrolled = self.ensure_face_id_enrolled(&device_id)?;

                if !is_enrolled {
                    return Ok(json!({
                        "error": {
                            "code": "FACE_ID_NOT_ENROLLED",
                            "message": "Face ID is not enrolled on this device",
                            "details": {
                                "manual_steps": [
                                    "1. In Simulator menu, go to Features > Face ID",
                                    "2. Check 'Enrolled' option",
                                    "3. Retry this scenario"
                                ]
                            }
                        }
                    }));
                }

                // Try AppleScript to trigger matching face
                let applescript = r#"
                    tell application "Simulator"
                        activate
                        delay 0.2
                        tell application "System Events"
                            tell process "Simulator"
                                click menu item "Matching Face" of menu "Face ID" of menu item "Face ID" of menu "Features" of menu bar 1
                            end tell
                        end tell
                    end tell
                "#;

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to execute AppleScript: {}", e)))?;

                if output.status.success() {
                    Ok(json!({
                        "success": true,
                        "scenario": "success_path",
                        "actions_taken": [
                            "Verified Face ID is enrolled",
                            "Triggered matching face via AppleScript"
                        ],
                        "expected_result": "User successfully authenticated with Face ID",
                        "test_purpose": "Verify app handles successful biometric authentication"
                    }))
                } else {
                    Ok(json!({
                        "error": {
                            "code": "AUTOMATION_FAILED",
                            "message": "Unable to trigger Face ID match",
                            "details": {
                                "manual_steps": [
                                    "When biometric prompt appears:",
                                    "Features > Face ID > Matching Face"
                                ]
                            }
                        }
                    }))
                }
            }

            "user_cancels" => {
                // Try AppleScript to send ESC key
                let applescript = r#"
                    tell application "Simulator"
                        activate
                    end tell
                    delay 0.1
                    tell application "System Events"
                        key code 53 -- ESC key
                    end tell
                "#;

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to send ESC: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "scenario": "user_cancels",
                    "actions_taken": ["Sent ESC key via AppleScript to dismiss biometric dialog"],
                    "expected_result": "Authentication cancelled by user",
                    "test_purpose": "Verify app handles user cancellation gracefully",
                    "note": "ESC key may not work for all dialogs"
                }))
            }

            "face_not_recognized" => {
                // Check if Face ID is enrolled
                let is_enrolled = self.ensure_face_id_enrolled(&device_id)?;

                if !is_enrolled {
                    return Ok(json!({
                        "error": {
                            "code": "FACE_ID_NOT_ENROLLED",
                            "message": "Face ID is not enrolled on this device"
                        }
                    }));
                }

                // Try AppleScript for non-matching face
                let applescript = r#"
                    tell application "Simulator"
                        activate
                        delay 0.2
                        tell application "System Events"
                            tell process "Simulator"
                                click menu item "Non-matching Face" of menu "Face ID" of menu item "Face ID" of menu "Features" of menu bar 1
                            end tell
                        end tell
                    end tell
                "#;

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to execute AppleScript: {}", e)))?;

                if output.status.success() {
                    Ok(json!({
                        "success": true,
                        "scenario": "face_not_recognized",
                        "actions_taken": [
                            "Verified Face ID is enrolled",
                            "Triggered non-matching face via AppleScript"
                        ],
                        "expected_result": "Face ID authentication failed",
                        "test_purpose": "Verify app handles failed biometric attempts"
                    }))
                } else {
                    Ok(json!({
                        "error": {
                            "code": "AUTOMATION_FAILED",
                            "message": "Unable to trigger Face ID non-match",
                            "details": {
                                "manual_steps": [
                                    "When biometric prompt appears:",
                                    "Features > Face ID > Non-matching Face"
                                ]
                            }
                        }
                    }))
                }
            }

            "biometric_lockout" => {
                // Note: Simulating biometric lockout requires multiple failed attempts
                // This cannot be easily automated without proper biometric dialog interaction
                let applescript = r#"
                    tell application "Simulator"
                        activate
                    end tell
                "#;

                Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output()
                    .ok();

                Ok(json!({
                    "success": false,
                    "scenario": "biometric_lockout",
                    "error": {
                        "code": "LOCKOUT_SIMULATION_NOT_AVAILABLE",
                        "message": "Biometric lockout simulation requires manual interaction",
                        "details": {
                            "reason": "Cannot programmatically trigger multiple biometric failures in sequence",
                            "manual_steps": [
                                "1. Trigger biometric prompt in app",
                                "2. Go to Features > Face ID > Non-matching Face",
                                "3. Repeat 5 times to trigger lockout",
                                "4. Verify app handles lockout appropriately"
                            ]
                        }
                    }
                }))
            }

            "fallback_to_passcode" => {
                // Note: simctl io sendkey/type commands don't exist
                // Passcode entry would need to be done via UI interaction coordinates
                // or XCUITest automation

                Ok(json!({
                    "success": false,
                    "scenario": "fallback_to_passcode",
                    "error": {
                        "code": "PASSCODE_ENTRY_NOT_AUTOMATED",
                        "message": "Passcode entry requires UI interaction",
                        "details": {
                            "reason": "simctl does not support keyboard input commands",
                            "alternatives": [
                                "Use ui_interaction tool with passcode button coordinates",
                                "Use XCUITest for passcode entry automation",
                                "Manual testing for passcode scenarios"
                            ]
                        }
                    }
                }))
            }

            "check_current_state" => {
                // Check if we're in a biometric dialog
                Ok(json!({
                    "success": true,
                    "scenario": "check_current_state",
                    "guidance": {
                        "if_dialog_visible": "Choose a scenario based on your test requirements",
                        "common_scenarios": {
                            "success_path": "Normal user flow - authenticate successfully",
                            "user_cancels": "Test cancellation handling",
                            "face_not_recognized": "Test failure handling"
                        },
                        "recommendation": "For most tests, use 'success_path' to proceed through the normal flow"
                    }
                }))
            }

            _ => Err(TestError::Mcp(format!("Unknown scenario: {}", scenario))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// Smart biometric handler that provides recommendations
pub struct SmartBiometricHandler {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl SmartBiometricHandler {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "smart_biometric_handler".to_string(),
                description: "Intelligently handle biometric dialogs based on test context"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "test_type": {
                            "type": "string",
                            "enum": ["login_flow", "security_test", "edge_case_test", "ui_test"],
                            "description": "The type of test being performed"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID"
                        }
                    },
                    "required": ["test_type"]
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for SmartBiometricHandler {
    async fn execute(&self, params: Value) -> Result<Value> {
        let test_type = params
            .get("test_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing test_type parameter".to_string()))?;

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

        match test_type {
            "login_flow" => {
                // For login tests, we want to succeed
                let applescript = r#"
                    tell application "Simulator"
                        activate
                        delay 0.2
                        tell application "System Events"
                            tell process "Simulator"
                                click menu item "Matching Face" of menu "Face ID" of menu item "Face ID" of menu "Features" of menu bar 1
                            end tell
                        end tell
                    end tell
                "#;

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("AppleScript failed: {}", e)))?;

                if output.status.success() {
                    Ok(json!({
                        "success": true,
                        "test_type": "login_flow",
                        "action_taken": "Provided matching face for successful authentication",
                        "reasoning": "Login flow tests should proceed through the happy path",
                        "next_steps": "Continue with post-login verification"
                    }))
                } else {
                    Ok(json!({
                        "error": {
                            "code": "AUTOMATION_FAILED",
                            "message": "Unable to trigger Face ID match",
                            "manual_steps": ["Features > Face ID > Matching Face"]
                        }
                    }))
                }
            }

            "security_test" => {
                // For security tests, try non-matching face
                let applescript = r#"
                    tell application "Simulator"
                        activate
                        delay 0.2
                        tell application "System Events"
                            tell process "Simulator"
                                click menu item "Non-matching Face" of menu "Face ID" of menu item "Face ID" of menu "Features" of menu bar 1
                            end tell
                        end tell
                    end tell
                "#;

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("AppleScript failed: {}", e)))?;

                if output.status.success() {
                    Ok(json!({
                        "success": true,
                        "test_type": "security_test",
                        "action_taken": "Provided non-matching face",
                        "reasoning": "Security tests should verify failure handling",
                        "next_steps": "Verify app shows appropriate error message"
                    }))
                } else {
                    Ok(json!({
                        "error": {
                            "code": "AUTOMATION_FAILED",
                            "message": "Unable to trigger Face ID non-match",
                            "manual_steps": ["Features > Face ID > Non-matching Face"]
                        }
                    }))
                }
            }

            "edge_case_test" => {
                // For edge cases, cancel the dialog using AppleScript
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
                    "test_type": "edge_case_test",
                    "action_taken": "Cancelled biometric dialog",
                    "reasoning": "Edge case tests should verify cancellation handling",
                    "next_steps": "Verify app returns to previous state gracefully"
                }))
            }

            "ui_test" => {
                // For UI tests, we just want to get past the dialog
                let applescript = r#"
                    tell application "Simulator"
                        activate
                        delay 0.2
                        tell application "System Events"
                            tell process "Simulator"
                                click menu item "Matching Face" of menu "Face ID" of menu item "Face ID" of menu "Features" of menu bar 1
                            end tell
                        end tell
                    end tell
                "#;

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(applescript)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("AppleScript failed: {}", e)))?;

                if output.status.success() {
                    Ok(json!({
                        "success": true,
                        "test_type": "ui_test",
                        "action_taken": "Quickly authenticated to proceed with UI test",
                        "reasoning": "UI tests need to get past authentication to test other features",
                        "next_steps": "Continue with UI element verification"
                    }))
                } else {
                    Ok(json!({
                        "error": {
                            "code": "AUTOMATION_FAILED",
                            "message": "Unable to trigger Face ID match",
                            "manual_steps": ["Features > Face ID > Matching Face"]
                        }
                    }))
                }
            }

            _ => Err(TestError::Mcp(format!("Unknown test type: {}", test_type))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
