use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

pub struct CoordinateValidator {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl CoordinateValidator {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "coordinate_validator".to_string(),
                description: "Validate and adjust coordinates for the current device screen bounds"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["validate", "adjust", "get_bounds"],
                            "description": "Action to perform"
                        },
                        "coordinates": {
                            "type": "object",
                            "properties": {
                                "x": {"type": "number"},
                                "y": {"type": "number"}
                            },
                            "required": ["x", "y"],
                            "description": "Coordinates to validate or adjust"
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

    fn get_device_bounds(&self, device_type: &str) -> (f64, f64) {
        // Return logical resolution for common devices
        match device_type {
            s if s.contains("iPhone-16-Pro-Max") || s.contains("iPhone 16 Pro Max") => {
                (430.0, 932.0)
            }
            s if s.contains("iPhone-16-Pro") || s.contains("iPhone 16 Pro") => (393.0, 852.0),
            s if s.contains("iPhone-16-Plus") || s.contains("iPhone 16 Plus") => (428.0, 926.0),
            s if s.contains("iPhone-16") || s.contains("iPhone 16") => (390.0, 844.0),
            s if s.contains("iPhone-15-Pro-Max") => (430.0, 932.0),
            s if s.contains("iPhone-15-Pro") => (393.0, 852.0),
            s if s.contains("iPhone-15-Plus") => (428.0, 926.0),
            s if s.contains("iPhone-15") => (390.0, 844.0),
            s if s.contains("iPhone-SE") => (375.0, 667.0),
            s if s.contains("iPad-Pro-13") => (1032.0, 1366.0),
            s if s.contains("iPad-Pro-11") => (820.0, 1180.0),
            s if s.contains("iPad") => (810.0, 1080.0),
            _ => (390.0, 844.0), // Default to iPhone 16 size
        }
    }

    fn validate_coordinates(&self, x: f64, y: f64, width: f64, height: f64) -> (bool, String) {
        let mut issues = Vec::new();

        if x < 0.0 {
            issues.push(format!("X coordinate {} is negative", x));
        }
        if y < 0.0 {
            issues.push(format!("Y coordinate {} is negative", y));
        }
        if x >= width {
            issues.push(format!("X coordinate {} exceeds screen width {}", x, width));
        }
        if y >= height {
            issues.push(format!(
                "Y coordinate {} exceeds screen height {}",
                y, height
            ));
        }

        let is_valid = issues.is_empty();
        let message = if is_valid {
            "Coordinates are within screen bounds".to_string()
        } else {
            issues.join(", ")
        };

        (is_valid, message)
    }

    fn adjust_coordinates(
        &self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> (f64, f64, Vec<String>) {
        let mut adjusted_x = x;
        let mut adjusted_y = y;
        let mut adjustments = Vec::new();

        // Clamp to screen bounds
        if x < 0.0 {
            adjusted_x = 0.0;
            adjustments.push(format!("X adjusted from {} to 0", x));
        } else if x >= width {
            adjusted_x = width - 1.0;
            adjustments.push(format!("X adjusted from {} to {}", x, adjusted_x));
        }

        if y < 0.0 {
            adjusted_y = 0.0;
            adjustments.push(format!("Y adjusted from {} to 0", y));
        } else if y >= height {
            adjusted_y = height - 1.0;
            adjustments.push(format!("Y adjusted from {} to {}", y, adjusted_y));
        }

        (adjusted_x, adjusted_y, adjustments)
    }
}

#[async_trait]
impl Tool for CoordinateValidator {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        // Get device info
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

        let device_info = self.device_manager.get_device(&device_id);
        let device_type = device_info
            .as_ref()
            .map(|d| d.device_type.as_str())
            .unwrap_or("unknown");

        let (width, height) = self.get_device_bounds(device_type);

        match action {
            "get_bounds" => Ok(json!({
                "success": true,
                "action": "get_bounds",
                "device_type": device_type,
                "device_id": device_id,
                "bounds": {
                    "width": width,
                    "height": height
                },
                "safe_area": {
                    "description": "Recommended tap area avoiding edges",
                    "min_x": 10,
                    "min_y": 50,
                    "max_x": width - 10.0,
                    "max_y": height - 50.0
                }
            })),
            "validate" => {
                let coordinates = params
                    .get("coordinates")
                    .ok_or_else(|| TestError::Mcp("Missing coordinates parameter".to_string()))?;

                let x = coordinates
                    .get("x")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| TestError::Mcp("Missing x coordinate".to_string()))?;

                let y = coordinates
                    .get("y")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| TestError::Mcp("Missing y coordinate".to_string()))?;

                let (is_valid, message) = self.validate_coordinates(x, y, width, height);

                Ok(json!({
                    "success": true,
                    "action": "validate",
                    "coordinates": {"x": x, "y": y},
                    "device_bounds": {"width": width, "height": height},
                    "is_valid": is_valid,
                    "message": message,
                    "device_type": device_type
                }))
            }
            "adjust" => {
                let coordinates = params
                    .get("coordinates")
                    .ok_or_else(|| TestError::Mcp("Missing coordinates parameter".to_string()))?;

                let x = coordinates
                    .get("x")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| TestError::Mcp("Missing x coordinate".to_string()))?;

                let y = coordinates
                    .get("y")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| TestError::Mcp("Missing y coordinate".to_string()))?;

                let (adjusted_x, adjusted_y, adjustments) =
                    self.adjust_coordinates(x, y, width, height);
                let was_adjusted = !adjustments.is_empty();

                Ok(json!({
                    "success": true,
                    "action": "adjust",
                    "original_coordinates": {"x": x, "y": y},
                    "adjusted_coordinates": {"x": adjusted_x, "y": adjusted_y},
                    "was_adjusted": was_adjusted,
                    "adjustments": adjustments,
                    "device_bounds": {"width": width, "height": height},
                    "device_type": device_type
                }))
            }
            _ => Err(TestError::Mcp(format!("Unsupported action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
