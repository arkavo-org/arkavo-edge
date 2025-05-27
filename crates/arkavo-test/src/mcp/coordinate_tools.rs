use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct CoordinateConverterKit {
    schema: ToolSchema,
}

impl CoordinateConverterKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "coordinate_converter".to_string(),
                description: "Convert between pixel and logical coordinates for iOS devices"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "x": {
                            "type": "number",
                            "description": "X coordinate to convert"
                        },
                        "y": {
                            "type": "number",
                            "description": "Y coordinate to convert"
                        },
                        "from": {
                            "type": "string",
                            "enum": ["pixels", "points"],
                            "description": "Source coordinate system"
                        },
                        "device_type": {
                            "type": "string",
                            "description": "Device type (e.g., 'iPhone 16 Pro Max'). If not specified, uses common scale factors."
                        }
                    },
                    "required": ["x", "y", "from"]
                }),
            },
        }
    }
}

impl Default for CoordinateConverterKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CoordinateConverterKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let x = params
            .get("x")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| TestError::Mcp("Missing x coordinate".to_string()))?;

        let y = params
            .get("y")
            .and_then(|v| v.as_f64())
            .ok_or_else(|| TestError::Mcp("Missing y coordinate".to_string()))?;

        let from = params
            .get("from")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing 'from' parameter".to_string()))?;

        let device_type = params
            .get("device_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Determine scale factor based on device
        let scale_factor = match device_type {
            s if s.contains("Pro Max") => 3.0,
            s if s.contains("Pro") => 3.0,
            s if s.contains("Plus") => 3.0,
            s if s.contains("iPhone 16") => 3.0,
            s if s.contains("iPhone 15") => 3.0,
            s if s.contains("iPhone 14") => 3.0,
            s if s.contains("iPhone 13") => 3.0,
            s if s.contains("iPhone 12") => 3.0,
            s if s.contains("iPhone 11") => 2.0,
            s if s.contains("iPhone SE") => 2.0,
            s if s.contains("iPad") => 2.0,
            _ => 3.0, // Default to 3x for modern devices
        };

        let (converted_x, converted_y, to_system) = match from {
            "pixels" => {
                // Convert pixels to points
                (x / scale_factor, y / scale_factor, "points")
            }
            "points" => {
                // Convert points to pixels
                (x * scale_factor, y * scale_factor, "pixels")
            }
            _ => {
                return Err(TestError::Mcp(
                    "Invalid 'from' value. Use 'pixels' or 'points'".to_string(),
                ));
            }
        };

        Ok(json!({
            "original": {
                "x": x,
                "y": y,
                "system": from
            },
            "converted": {
                "x": converted_x,
                "y": converted_y,
                "system": to_system
            },
            "scale_factor": scale_factor,
            "device_info": {
                "device_type": if device_type.is_empty() { "Generic iOS Device" } else { device_type },
                "note": "iOS uses logical coordinates (points) for UI elements. Retina displays have 2x or 3x scale factors."
            }
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
