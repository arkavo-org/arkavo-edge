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
        // Get device dimensions and scale factor
        let (scale_factor, logical_width, logical_height) = match device_type {
            s if s.contains("iPhone") && s.contains("Pro Max") => (3.0, 430.0, 932.0),
            s if s.contains("iPhone") && s.contains("Pro") => (3.0, 393.0, 852.0),
            s if s.contains("iPhone") && s.contains("Plus") => (3.0, 428.0, 926.0),
            s if s.contains("iPhone 16") => (3.0, 390.0, 844.0),
            s if s.contains("iPhone 15") => (3.0, 390.0, 844.0),
            s if s.contains("iPhone 14") => (3.0, 390.0, 844.0),
            s if s.contains("iPhone 13") => (3.0, 390.0, 844.0),
            s if s.contains("iPhone 12") => (3.0, 390.0, 844.0),
            s if s.contains("iPhone 11") => (2.0, 414.0, 896.0),
            s if s.contains("iPhone SE") => (2.0, 375.0, 667.0),
            s if s.contains("iPad") && s.contains("Pro") && s.contains("13") => (2.0, 1024.0, 1366.0),
            s if s.contains("iPad") && s.contains("Pro") && s.contains("11") => (2.0, 834.0, 1194.0),
            s if s.contains("iPad") => (2.0, 820.0, 1180.0),
            _ => (3.0, 393.0, 852.0), // Default to iPhone Pro size
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

        // Check if coordinates are within bounds
        let mut warnings = Vec::new();
        
        if from == "points" {
            if x < 0.0 || x > logical_width {
                warnings.push(format!("X coordinate {} is out of bounds (0-{})", x, logical_width));
            }
            if y < 0.0 || y > logical_height {
                warnings.push(format!("Y coordinate {} is out of bounds (0-{})", y, logical_height));
            }
        } else if from == "pixels" {
            let pixel_width = logical_width * scale_factor;
            let pixel_height = logical_height * scale_factor;
            if x < 0.0 || x > pixel_width {
                warnings.push(format!("X coordinate {} is out of bounds (0-{})", x, pixel_width));
            }
            if y < 0.0 || y > pixel_height {
                warnings.push(format!("Y coordinate {} is out of bounds (0-{})", y, pixel_height));
            }
        }

        let mut result = json!({
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
                "logical_resolution": {
                    "width": logical_width,
                    "height": logical_height
                },
                "pixel_resolution": {
                    "width": logical_width * scale_factor,
                    "height": logical_height * scale_factor
                },
                "note": "iOS uses logical coordinates (points) for UI elements. Retina displays have 2x or 3x scale factors."
            }
        });

        if !warnings.is_empty() {
            result["warnings"] = json!(warnings);
            result["valid_bounds"] = json!({
                "points": {
                    "x": format!("0-{}", logical_width),
                    "y": format!("0-{}", logical_height)
                },
                "pixels": {
                    "x": format!("0-{}", logical_width * scale_factor),
                    "y": format!("0-{}", logical_height * scale_factor)
                }
            });
        }

        Ok(result)
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
