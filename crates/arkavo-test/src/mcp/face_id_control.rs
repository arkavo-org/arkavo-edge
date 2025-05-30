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
                // Enable Face ID enrollment
                let output = Command::new("xcrun")
                    .args([
                        "simctl",
                        "spawn",
                        &device_id,
                        "notifyutil",
                        "-s",
                        "com.apple.BiometricKit.enrollmentChanged",
                        "1",
                    ])
                    .output()
                    .map_err(|e| {
                        TestError::Mcp(format!("Failed to set Face ID enrollment: {}", e))
                    })?;

                if output.status.success() {
                    // Also use the UI command for enrollment
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

                    Ok(json!({
                        "success": true,
                        "action": "enroll",
                        "device_id": device_id,
                        "message": "Face ID enrolled successfully"
                    }))
                } else {
                    Ok(json!({
                        "success": false,
                        "error": String::from_utf8_lossy(&output.stderr).trim().to_string()
                    }))
                }
            }
            "unenroll" => {
                // Disable Face ID enrollment
                let output = Command::new("xcrun")
                    .args([
                        "simctl",
                        "spawn",
                        &device_id,
                        "notifyutil",
                        "-s",
                        "com.apple.BiometricKit.enrollmentChanged",
                        "0",
                    ])
                    .output()
                    .map_err(|e| {
                        TestError::Mcp(format!("Failed to clear Face ID enrollment: {}", e))
                    })?;

                if output.status.success() {
                    // Also use the UI command for unenrollment
                    Command::new("xcrun")
                        .args([
                            "simctl",
                            "ui",
                            &device_id,
                            "biometric",
                            "enrollment",
                            "--cleared",
                        ])
                        .output()
                        .ok();

                    Ok(json!({
                        "success": true,
                        "action": "unenroll",
                        "device_id": device_id,
                        "message": "Face ID enrollment cleared"
                    }))
                } else {
                    Ok(json!({
                        "success": false,
                        "error": String::from_utf8_lossy(&output.stderr).trim().to_string()
                    }))
                }
            }
            "match" => {
                // Simulate successful Face ID match (Matching Face)
                let output = Command::new("xcrun")
                    .args(["simctl", "ui", &device_id, "biometric", "match"])
                    .output()
                    .map_err(|e| {
                        TestError::Mcp(format!("Failed to simulate Face ID match: {}", e))
                    })?;

                if output.status.success() {
                    Ok(json!({
                        "success": true,
                        "action": "match",
                        "device_id": device_id,
                        "message": "Face ID authentication successful (Matching Face)"
                    }))
                } else {
                    Ok(json!({
                        "success": false,
                        "error": String::from_utf8_lossy(&output.stderr).trim().to_string(),
                        "suggestion": "Make sure Face ID is enrolled first using 'enroll' action"
                    }))
                }
            }
            "no_match" => {
                // Simulate failed Face ID match (Non-matching Face)
                let output = Command::new("xcrun")
                    .args(["simctl", "ui", &device_id, "biometric", "nomatch"])
                    .output()
                    .map_err(|e| {
                        TestError::Mcp(format!("Failed to simulate Face ID no match: {}", e))
                    })?;

                if output.status.success() {
                    Ok(json!({
                        "success": true,
                        "action": "no_match",
                        "device_id": device_id,
                        "message": "Face ID authentication failed (Non-matching Face)"
                    }))
                } else {
                    Ok(json!({
                        "success": false,
                        "error": String::from_utf8_lossy(&output.stderr).trim().to_string()
                    }))
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
                "enroll": "Enable Face ID (like selecting 'Enrolled' in simulator menu)",
                "unenroll": "Disable Face ID (clear enrollment)",
                "match": "Simulate successful Face ID scan (like selecting 'Matching Face')",
                "no_match": "Simulate failed Face ID scan (like selecting 'Non-matching Face')"
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
