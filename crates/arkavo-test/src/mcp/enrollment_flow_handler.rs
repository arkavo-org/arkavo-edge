use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct EnrollmentFlowHandler {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl EnrollmentFlowHandler {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "enrollment_flow".to_string(),
                description:
                    "Complete biometric enrollment flow including dialog handling and app relaunch"
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["complete_enrollment", "dismiss_and_relaunch", "enroll_and_continue"],
                            "description": "Action to perform for enrollment flow"
                        },
                        "app_bundle_id": {
                            "type": "string",
                            "description": "Bundle ID of the app to relaunch (default: com.arkavo.app)"
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

    fn dismiss_dialog(&self) -> Result<()> {
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
            .map_err(|e| TestError::Mcp(format!("Failed to dismiss dialog: {}", e)))?;

        Ok(())
    }

    fn terminate_app(&self, device_id: &str, bundle_id: &str) -> Result<()> {
        let output = Command::new("xcrun")
            .arg("simctl")
            .arg("terminate")
            .arg(device_id)
            .arg(bundle_id)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to terminate app: {}", e)))?;

        if !output.status.success() {
            // App might not be running, which is okay
            eprintln!("Note: App might not have been running");
        }

        Ok(())
    }

    fn launch_app(&self, device_id: &str, bundle_id: &str) -> Result<()> {
        let output = Command::new("xcrun")
            .arg("simctl")
            .arg("launch")
            .arg(device_id)
            .arg(bundle_id)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to launch app: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to launch app: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    fn enroll_biometrics(&self, _device_id: &str) -> Result<()> {
        // Use AppleScript to enable Face ID enrollment
        let script = r#"tell application "Simulator"
            activate
        end tell
        tell application "System Events"
            tell process "Simulator"
                set frontmost to true
                click menu item "Face ID" of menu "Features" of menu bar 1
                delay 0.5
                click menu item "Enrolled" of menu 1 of menu item "Face ID" of menu "Features" of menu bar 1
            end tell
        end tell"#;

        let output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to enroll biometrics: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to enroll biometrics: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl Tool for EnrollmentFlowHandler {
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

        let bundle_id = params
            .get("app_bundle_id")
            .and_then(|v| v.as_str())
            .unwrap_or("com.arkavo.app");

        match action {
            "complete_enrollment" => {
                // Complete enrollment flow
                let mut steps_completed = vec![];

                // Step 1: Dismiss dialog
                if self.dismiss_dialog().is_ok() {
                    steps_completed.push("Dismissed enrollment dialog");
                    thread::sleep(Duration::from_millis(500));
                }

                // Step 2: Enroll biometrics
                match self.enroll_biometrics(&device_id) {
                    Ok(_) => {
                        steps_completed.push("Enrolled Face ID");
                        thread::sleep(Duration::from_millis(1000));
                    }
                    Err(e) => {
                        return Ok(json!({
                            "success": false,
                            "error": {
                                "code": "ENROLLMENT_FAILED",
                                "message": e.to_string(),
                                "steps_completed": steps_completed
                            }
                        }));
                    }
                }

                // Step 3: Relaunch app
                self.terminate_app(&device_id, bundle_id).ok();
                thread::sleep(Duration::from_millis(500));

                match self.launch_app(&device_id, bundle_id) {
                    Ok(_) => {
                        steps_completed.push("Relaunched app");
                        Ok(json!({
                            "success": true,
                            "action": "complete_enrollment",
                            "device_id": device_id,
                            "app_bundle_id": bundle_id,
                            "steps_completed": steps_completed,
                            "message": "Enrollment flow completed successfully"
                        }))
                    }
                    Err(e) => Ok(json!({
                        "success": false,
                        "error": {
                            "code": "RELAUNCH_FAILED",
                            "message": e.to_string(),
                            "steps_completed": steps_completed
                        }
                    })),
                }
            }
            "dismiss_and_relaunch" => {
                // Just dismiss and relaunch without enrollment
                let mut steps_completed = vec![];

                // Dismiss dialog
                if self.dismiss_dialog().is_ok() {
                    steps_completed.push("Dismissed enrollment dialog");
                    thread::sleep(Duration::from_millis(500));
                }

                // Terminate and relaunch
                self.terminate_app(&device_id, bundle_id).ok();
                thread::sleep(Duration::from_millis(500));

                match self.launch_app(&device_id, bundle_id) {
                    Ok(_) => {
                        steps_completed.push("Relaunched app");
                        Ok(json!({
                            "success": true,
                            "action": "dismiss_and_relaunch",
                            "device_id": device_id,
                            "app_bundle_id": bundle_id,
                            "steps_completed": steps_completed
                        }))
                    }
                    Err(e) => Ok(json!({
                        "success": false,
                        "error": {
                            "code": "RELAUNCH_FAILED",
                            "message": e.to_string(),
                            "steps_completed": steps_completed
                        }
                    })),
                }
            }
            "enroll_and_continue" => {
                // Enroll biometrics without dismissing dialog or relaunching
                match self.enroll_biometrics(&device_id) {
                    Ok(_) => Ok(json!({
                        "success": true,
                        "action": "enroll_and_continue",
                        "device_id": device_id,
                        "message": "Face ID enrolled, app should continue normally"
                    })),
                    Err(e) => Ok(json!({
                        "success": false,
                        "error": {
                            "code": "ENROLLMENT_FAILED",
                            "message": e.to_string()
                        }
                    })),
                }
            }
            _ => Err(TestError::Mcp(format!("Unsupported action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
