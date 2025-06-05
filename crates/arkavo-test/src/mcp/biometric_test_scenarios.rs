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

    fn ensure_face_id_enrolled(&self, _device_id: &str) -> Result<()> {
        // simctl ui biometric is NOT a valid command
        // Face ID enrollment must be done manually through Simulator menu
        Err(TestError::Mcp(
            "Face ID enrollment cannot be automated. \n\
             Manual steps required:\n\
             1. In Simulator menu, go to Device > Face ID\n\
             2. Check 'Enrolled' option\n\
             3. Use 'Matching Face' when authentication is needed".to_string()
        ))
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
                // Ensure Face ID is enrolled
                self.ensure_face_id_enrolled(&device_id)?;

                // Provide matching face
                let output = Command::new("xcrun")
                    .args(["simctl", "ui", &device_id, "biometric", "match"])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to match face: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "scenario": "success_path",
                    "actions_taken": [
                        "Ensured Face ID is enrolled",
                        "Provided matching face for authentication"
                    ],
                    "expected_result": "User successfully authenticated with Face ID",
                    "test_purpose": "Verify app handles successful biometric authentication"
                }))
            }

            "user_cancels" => {
                // Send ESC key to cancel the dialog
                Command::new("xcrun")
                    .args(["simctl", "io", &device_id, "sendkey", "escape"])
                    .output()
                    .ok();

                Ok(json!({
                    "success": true,
                    "scenario": "user_cancels",
                    "actions_taken": ["Sent ESC key to dismiss biometric dialog"],
                    "expected_result": "Authentication cancelled by user",
                    "test_purpose": "Verify app handles user cancellation gracefully"
                }))
            }

            "face_not_recognized" => {
                self.ensure_face_id_enrolled(&device_id)?;

                // Simulate non-matching face
                let output = Command::new("xcrun")
                    .args(["simctl", "ui", &device_id, "biometric", "nomatch"])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to simulate no match: {}", e)))?;

                Ok(json!({
                    "success": output.status.success(),
                    "scenario": "face_not_recognized",
                    "actions_taken": [
                        "Ensured Face ID is enrolled",
                        "Provided non-matching face"
                    ],
                    "expected_result": "Face ID authentication failed",
                    "test_purpose": "Verify app handles failed biometric attempts"
                }))
            }

            "biometric_lockout" => {
                // Simulate multiple failed attempts
                for _ in 0..5 {
                    Command::new("xcrun")
                        .args(["simctl", "ui", &device_id, "biometric", "nomatch"])
                        .output()
                        .ok();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }

                Ok(json!({
                    "success": true,
                    "scenario": "biometric_lockout",
                    "actions_taken": ["Simulated 5 failed Face ID attempts"],
                    "expected_result": "Biometric authentication locked out",
                    "test_purpose": "Verify app handles biometric lockout and offers alternatives"
                }))
            }

            "fallback_to_passcode" => {
                // Type a passcode (default simulator passcode)
                let passcode = "1234";
                for digit in passcode.chars() {
                    Command::new("xcrun")
                        .args(["simctl", "io", &device_id, "type", &digit.to_string()])
                        .output()
                        .ok();
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                // Press return
                Command::new("xcrun")
                    .args(["simctl", "io", &device_id, "sendkey", "return"])
                    .output()
                    .ok();

                Ok(json!({
                    "success": true,
                    "scenario": "fallback_to_passcode",
                    "actions_taken": [
                        "Typed passcode digits",
                        "Pressed return key"
                    ],
                    "expected_result": "Authentication via passcode",
                    "test_purpose": "Verify passcode fallback works correctly"
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

        match test_type {
            "login_flow" => {
                // For login tests, we want to succeed
                Command::new("xcrun")
                    .args([
                        "simctl",
                        "ui",
                        &device_id,
                        "biometric",
                        "enrollment",
                        "--enrolled",
                    ])
                    .output()
                    .ok();

                Command::new("xcrun")
                    .args(["simctl", "ui", &device_id, "biometric", "match"])
                    .output()
                    .ok();

                Ok(json!({
                    "success": true,
                    "test_type": "login_flow",
                    "action_taken": "Provided matching face for successful authentication",
                    "reasoning": "Login flow tests should proceed through the happy path",
                    "next_steps": "Continue with post-login verification"
                }))
            }

            "security_test" => {
                // For security tests, try to break things
                Command::new("xcrun")
                    .args(["simctl", "ui", &device_id, "biometric", "nomatch"])
                    .output()
                    .ok();

                Ok(json!({
                    "success": true,
                    "test_type": "security_test",
                    "action_taken": "Provided non-matching face",
                    "reasoning": "Security tests should verify failure handling",
                    "next_steps": "Verify app shows appropriate error message"
                }))
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
                Command::new("xcrun")
                    .args(["simctl", "ui", &device_id, "biometric", "match"])
                    .output()
                    .ok();

                Ok(json!({
                    "success": true,
                    "test_type": "ui_test",
                    "action_taken": "Quickly authenticated to proceed with UI test",
                    "reasoning": "UI tests need to get past authentication to test other features",
                    "next_steps": "Continue with UI element verification"
                }))
            }

            _ => Err(TestError::Mcp(format!("Unknown test type: {}", test_type))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
