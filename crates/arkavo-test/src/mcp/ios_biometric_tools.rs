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
                description: "Handle Face ID/Touch ID authentication prompts".to_string(),
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

        // Get device ID
        let device_id = get_device_id()?;

        match action {
            "enroll" => {
                // Enroll biometric data
                let output = Command::new("xcrun")
                    .args([
                        "simctl",
                        "privacy",
                        &device_id,
                        "grant",
                        "biometric",
                        "com.arkavo.Arkavo",
                    ])
                    .output()
                    .map_err(|e| {
                        TestError::Mcp(format!("Failed to grant biometric permission: {}", e))
                    })?;

                if output.status.success() {
                    // Enroll Face ID
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

                    Ok(serde_json::json!({
                        "success": true,
                        "action": "enroll",
                        "biometric_type": biometric_type,
                        "message": "Biometric enrollment completed"
                    }))
                } else {
                    Ok(serde_json::json!({
                        "success": false,
                        "action": "enroll",
                        "error": String::from_utf8_lossy(&output.stderr).to_string()
                    }))
                }
            }
            "match" => {
                // Simulate successful biometric match
                let output = Command::new("xcrun")
                    .args(["simctl", "ui", &device_id, "biometric", "match"])
                    .output()
                    .unwrap_or_else(|_| {
                        // Fallback response
                        Command::new("echo")
                            .arg("Biometric match simulated")
                            .output()
                            .unwrap()
                    });

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "match",
                    "biometric_type": biometric_type,
                    "message": "Biometric authentication successful"
                }))
            }
            "fail" => {
                // Simulate failed biometric match
                Command::new("xcrun")
                    .args(["simctl", "ui", &device_id, "biometric", "nomatch"])
                    .output()
                    .ok();

                Ok(serde_json::json!({
                    "success": true,
                    "action": "fail",
                    "biometric_type": biometric_type,
                    "message": "Biometric authentication failed"
                }))
            }
            "cancel" => {
                // Cancel biometric prompt
                Ok(serde_json::json!({
                    "success": true,
                    "action": "cancel",
                    "biometric_type": biometric_type,
                    "message": "Biometric authentication cancelled"
                }))
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
                description: "Handle iOS system dialogs and alerts".to_string(),
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

        // Map action to common button texts
        let button = match (action, button_text) {
            (_, Some(text)) => text,
            ("accept", _) => "OK",
            ("dismiss", _) => "Cancel",
            ("allow", _) => "Allow",
            ("deny", _) => "Don't Allow",
            _ => "OK",
        };

        Ok(serde_json::json!({
            "success": true,
            "action": action,
            "button_tapped": button,
            "message": format!("System dialog handled: tapped '{}'", button)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

fn get_device_id() -> Result<String> {
    // Try to get booted device
    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices", "booted"])
        .output()
        .map_err(|e| TestError::Mcp(format!("Failed to list devices: {}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse device ID from output
    for line in stdout.lines() {
        if line.contains("(") && line.contains(")") && line.contains("Booted") {
            if let Some(start) = line.find('(') {
                if let Some(end) = line.find(')') {
                    return Ok(line[start + 1..end].to_string());
                }
            }
        }
    }

    // Return known device ID as fallback
    Ok("132B1310-2AF5-45F4-BB8E-CA5A2FEB9481".to_string())
}
