use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct EnrollmentDialogHandler {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl EnrollmentDialogHandler {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "enrollment_dialog".to_string(),
                description: "Handle the 'Simulator requires enrolled biometrics to use passkeys' dialog with device-specific coordinates".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["get_cancel_coordinates", "tap_cancel"],
                            "description": "Action to perform on enrollment dialog"
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

    fn get_cancel_coordinates(&self, device_type: &str) -> (f64, f64) {
        // Device-specific coordinates for the Cancel button
        // Based on actual observations from screenshots
        match device_type {
            s if s.contains("iPhone-16-Pro-Max") || s.contains("iPhone 16 Pro Max") => {
                // iPhone 16 Pro Max: 430x932 logical resolution
                // Cancel button is centered horizontally at bottom of dialog
                (215.0, 830.0)
            }
            s if s.contains("iPhone-16-Pro") || s.contains("iPhone 16 Pro") => {
                // iPhone 16 Pro: 393x852 logical resolution
                (196.5, 750.0)
            }
            s if s.contains("iPhone-16-Plus") || s.contains("iPhone 16 Plus") => {
                // iPhone 16 Plus: 428x926 logical resolution
                (214.0, 820.0)
            }
            s if s.contains("iPhone-16") || s.contains("iPhone 16") => {
                // iPhone 16: 390x844 logical resolution
                (195.0, 740.0)
            }
            s if s.contains("iPhone-15") => {
                // iPhone 15 variants have similar layouts
                if s.contains("Pro-Max") {
                    (215.0, 830.0)
                } else if s.contains("Pro") {
                    (196.5, 750.0)
                } else if s.contains("Plus") {
                    (214.0, 820.0)
                } else {
                    (195.0, 740.0)
                }
            }
            s if s.contains("iPhone-SE") => {
                // iPhone SE: 375x667 logical resolution
                (187.5, 550.0)
            }
            s if s.contains("iPad") => {
                // iPad: 820x1180 logical resolution (typical)
                (410.0, 900.0)
            }
            _ => {
                // Default to iPhone Pro size
                (196.5, 750.0)
            }
        }
    }
}

#[async_trait]
impl Tool for EnrollmentDialogHandler {
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

        // Get device info
        let device_info = self.device_manager.get_device(&device_id);
        let device_type = device_info
            .as_ref()
            .map(|d| d.device_type.as_str())
            .unwrap_or("unknown");

        match action {
            "get_cancel_coordinates" => {
                let (x, y) = self.get_cancel_coordinates(device_type);

                Ok(json!({
                    "success": true,
                    "action": "get_cancel_coordinates",
                    "device_type": device_type,
                    "cancel_button": {
                        "x": x,
                        "y": y,
                        "description": "Center of Cancel button in enrollment dialog"
                    },
                    "usage": {
                        "description": "Use these coordinates with ui_interaction tool",
                        "example": {
                            "tool": "ui_interaction",
                            "params": {
                                "action": "tap",
                                "target": {
                                    "x": x,
                                    "y": y
                                }
                            }
                        }
                    }
                }))
            }
            "tap_cancel" => {
                let (x, y) = self.get_cancel_coordinates(device_type);

                Ok(json!({
                    "success": true,
                    "action": "tap_cancel",
                    "message": "To tap the Cancel button, use ui_interaction tool with the provided coordinates",
                    "coordinates": {
                        "x": x,
                        "y": y
                    },
                    "next_step": {
                        "tool": "ui_interaction",
                        "params": {
                            "action": "tap",
                            "target": {
                                "x": x,
                                "y": y
                            }
                        }
                    }
                }))
            }
            _ => Err(TestError::Mcp(format!("Unsupported action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
