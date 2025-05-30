use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;

pub struct IdbUiKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

#[derive(Debug, Deserialize)]
struct UiElement {
    #[serde(rename = "AXLabel")]
    ax_label: Option<String>,
    #[serde(rename = "AXValue")]
    ax_value: Option<String>,
    #[serde(rename = "AXPlaceholderValue")]
    ax_placeholder: Option<String>,
    #[serde(rename = "type")]
    element_type: Option<String>,
    frame: Frame,
    #[serde(rename = "AXEnabled")]
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize)]
struct Frame {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl IdbUiKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "idb_ui".to_string(),
                description: "IDB-based UI interaction for accurate element targeting".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["describe_ui", "tap_element", "dismiss_dialog"],
                            "description": "The UI action to perform"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "element_label": {
                            "type": "string",
                            "description": "The accessibility label of the element to interact with (for tap_element)"
                        },
                        "element_type": {
                            "type": "string",
                            "description": "Optional element type filter (Button, TextField, etc.)"
                        }
                    },
                    "required": ["action"]
                }),
            },
            device_manager,
        }
    }

    fn get_ui_elements(&self, device_id: &str) -> Result<Vec<UiElement>> {
        let output = Command::new("idb")
            .args(["ui", "describe-all", "--udid", device_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run idb ui describe-all: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "idb ui describe-all failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Parse JSON output
        serde_json::from_str(&stdout)
            .map_err(|e| TestError::Mcp(format!("Failed to parse UI elements: {}", e)))
    }

    fn tap_at_coordinates(&self, device_id: &str, x: f64, y: f64) -> Result<()> {
        let output = Command::new("idb")
            .args([
                "ui",
                "tap",
                &x.to_string(),
                &y.to_string(),
                "--udid",
                device_id,
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run idb ui tap: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "idb ui tap failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }
}

#[async_trait]
impl Tool for IdbUiKit {
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
            "describe_ui" => {
                match self.get_ui_elements(&device_id) {
                    Ok(elements) => {
                        // Filter and format elements for easier reading
                        let formatted_elements: Vec<_> = elements
                            .iter()
                            .filter(|e| e.ax_label.is_some() || e.ax_value.is_some())
                            .map(|e| {
                                json!({
                                    "type": e.element_type,
                                    "label": e.ax_label,
                                    "value": e.ax_value,
                                    "placeholder": e.ax_placeholder,
                                    "frame": {
                                        "x": e.frame.x,
                                        "y": e.frame.y,
                                        "width": e.frame.width,
                                        "height": e.frame.height,
                                        "center": {
                                            "x": e.frame.x + e.frame.width / 2.0,
                                            "y": e.frame.y + e.frame.height / 2.0
                                        }
                                    },
                                    "enabled": e.enabled
                                })
                            })
                            .collect();

                        Ok(json!({
                            "success": true,
                            "action": "describe_ui",
                            "device_id": device_id,
                            "element_count": formatted_elements.len(),
                            "elements": formatted_elements
                        }))
                    }
                    Err(e) => Ok(json!({
                        "success": false,
                        "error": e.to_string(),
                        "suggestion": "Make sure IDB is properly installed and the device is connected"
                    })),
                }
            }
            "tap_element" => {
                let element_label = params
                    .get("element_label")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing element_label parameter".to_string()))?;

                let element_type_filter = params.get("element_type").and_then(|v| v.as_str());

                match self.get_ui_elements(&device_id) {
                    Ok(elements) => {
                        // Find the element by label
                        let matching_element = elements.iter().find(|e| {
                            let label_match = e
                                .ax_label
                                .as_ref()
                                .map(|l| l.contains(element_label))
                                .unwrap_or(false);

                            let type_match = element_type_filter
                                .map(|t| e.element_type.as_ref().map(|et| et == t).unwrap_or(false))
                                .unwrap_or(true);

                            label_match && type_match
                        });

                        if let Some(element) = matching_element {
                            // Calculate center coordinates
                            let center_x = element.frame.x + element.frame.width / 2.0;
                            let center_y = element.frame.y + element.frame.height / 2.0;

                            // Tap at center
                            match self.tap_at_coordinates(&device_id, center_x, center_y) {
                                Ok(_) => Ok(json!({
                                    "success": true,
                                    "action": "tap_element",
                                    "element_label": element_label,
                                    "tapped_at": {
                                        "x": center_x,
                                        "y": center_y
                                    },
                                    "element": {
                                        "type": element.element_type,
                                        "frame": element.frame,
                                        "enabled": element.enabled
                                    }
                                })),
                                Err(e) => Ok(json!({
                                    "success": false,
                                    "error": e.to_string()
                                })),
                            }
                        } else {
                            Ok(json!({
                                "success": false,
                                "error": format!("No element found with label containing '{}'", element_label),
                                "suggestion": "Use describe_ui action to see available elements"
                            }))
                        }
                    }
                    Err(e) => Ok(json!({
                        "success": false,
                        "error": e.to_string()
                    })),
                }
            }
            "dismiss_dialog" => {
                match self.get_ui_elements(&device_id) {
                    Ok(elements) => {
                        // Look for common dismiss buttons
                        let dismiss_labels = ["Cancel", "X", "Close", "Dismiss", "OK", "Done"];

                        let dismiss_element = elements.iter().find(|e| {
                            if let Some(label) = &e.ax_label {
                                dismiss_labels.iter().any(|&dl| label.contains(dl))
                            } else {
                                false
                            }
                        });

                        if let Some(element) = dismiss_element {
                            // Calculate center coordinates
                            let center_x = element.frame.x + element.frame.width / 2.0;
                            let center_y = element.frame.y + element.frame.height / 2.0;

                            match self.tap_at_coordinates(&device_id, center_x, center_y) {
                                Ok(_) => Ok(json!({
                                    "success": true,
                                    "action": "dismiss_dialog",
                                    "dismissed_element": element.ax_label,
                                    "tapped_at": {
                                        "x": center_x,
                                        "y": center_y
                                    }
                                })),
                                Err(e) => Ok(json!({
                                    "success": false,
                                    "error": e.to_string()
                                })),
                            }
                        } else {
                            // Try tapping outside the dialog (backdrop)
                            // Typically safe areas are top corners
                            match self.tap_at_coordinates(&device_id, 50.0, 50.0) {
                                Ok(_) => Ok(json!({
                                    "success": true,
                                    "action": "dismiss_dialog",
                                    "method": "backdrop_tap",
                                    "tapped_at": {
                                        "x": 50.0,
                                        "y": 50.0
                                    }
                                })),
                                Err(e) => Ok(json!({
                                    "success": false,
                                    "error": "No dismiss button found and backdrop tap failed",
                                    "details": e.to_string()
                                })),
                            }
                        }
                    }
                    Err(e) => Ok(json!({
                        "success": false,
                        "error": e.to_string()
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
