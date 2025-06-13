use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use super::simulator_interaction::SimulatorInteraction;
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct UiElementHandler {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
    simulator_interaction: SimulatorInteraction,
}

impl UiElementHandler {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            simulator_interaction: SimulatorInteraction::new(),
            schema: ToolSchema {
                name: "ui_element_handler".to_string(),
                description: "Advanced UI element interaction with multiple strategies for checkboxes, switches, and other controls".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["tap_checkbox", "tap_switch", "double_tap", "long_press", "tap_with_retry"],
                            "description": "Type of interaction to perform"
                        },
                        "coordinates": {
                            "type": "object",
                            "properties": {
                                "x": {"type": "number"},
                                "y": {"type": "number"}
                            },
                            "required": ["x", "y"],
                            "description": "Target coordinates for the interaction"
                        },
                        "retry_count": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 5,
                            "default": 3,
                            "description": "Number of retry attempts for tap_with_retry"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        }
                    },
                    "required": ["action", "coordinates"]
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

    #[allow(dead_code)]
    async fn perform_tap_async(&self, device_id: &str, x: f64, y: f64) -> Result<()> {
        // Use the new version-aware simulator interaction
        match self.simulator_interaction.tap(device_id, x, y).await {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn perform_tap(&self, x: f64, y: f64) -> Result<()> {
        // Use AppleScript for reliable tapping
        let script = format!(
            r#"tell application "Simulator"
                activate
            end tell
            delay 0.1
            tell application "System Events"
                tell process "Simulator"
                    click at {{{}, {}}}
                end tell
            end tell"#,
            x, y
        );

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute tap: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Tap failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    fn perform_double_tap(&self, x: f64, y: f64) -> Result<()> {
        // Perform two taps in quick succession
        self.perform_tap(x, y)?;
        thread::sleep(Duration::from_millis(100));
        self.perform_tap(x, y)?;
        Ok(())
    }

    fn perform_long_press(&self, x: f64, y: f64) -> Result<()> {
        // Use AppleScript for long press
        let script = format!(
            r#"tell application "Simulator"
                activate
            end tell
            delay 0.1
            tell application "System Events"
                tell process "Simulator"
                    set frontmost to true
                    click at {{{}, {}}} with pressing
                end tell
            end tell"#,
            x, y
        );

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute long press: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Long press failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    fn tap_checkbox_with_strategies(&self, x: f64, y: f64) -> Result<Vec<String>> {
        let mut strategies_tried = Vec::new();

        // Strategy 1: Direct tap on checkbox
        strategies_tried.push("direct_tap".to_string());
        if self.perform_tap(x, y).is_ok() {
            thread::sleep(Duration::from_millis(300));
            return Ok(strategies_tried);
        }

        // Strategy 2: Tap slightly to the left (for checkboxes with labels)
        strategies_tried.push("tap_left_offset".to_string());
        if self.perform_tap(x - 10.0, y).is_ok() {
            thread::sleep(Duration::from_millis(300));
            return Ok(strategies_tried);
        }

        // Strategy 3: Double tap (some UI frameworks require this)
        strategies_tried.push("double_tap".to_string());
        if self.perform_double_tap(x, y).is_ok() {
            thread::sleep(Duration::from_millis(300));
            return Ok(strategies_tried);
        }

        // Strategy 4: Tap with slight offset in all directions
        for (dx, dy, label) in &[
            (0.0, -5.0, "tap_up"),
            (0.0, 5.0, "tap_down"),
            (-5.0, 0.0, "tap_left"),
            (5.0, 0.0, "tap_right"),
        ] {
            strategies_tried.push(label.to_string());
            if self.perform_tap(x + dx, y + dy).is_ok() {
                thread::sleep(Duration::from_millis(300));
                return Ok(strategies_tried);
            }
        }

        Err(TestError::Mcp(
            "All checkbox tap strategies failed".to_string(),
        ))
    }
}

#[async_trait]
impl Tool for UiElementHandler {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

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

        // Check Xcode version compatibility
        let version_info = self.simulator_interaction.get_version_info();
        eprintln!("Using Xcode version info: {}", version_info);

        match action {
            "tap_checkbox" => match self.tap_checkbox_with_strategies(x, y) {
                Ok(strategies) => Ok(json!({
                    "success": true,
                    "action": "tap_checkbox",
                    "coordinates": {"x": x, "y": y},
                    "device_id": device_id,
                    "strategies_tried": strategies,
                    "message": "Checkbox tapped using multiple strategies"
                })),
                Err(e) => Ok(json!({
                    "success": false,
                    "error": {
                        "code": "CHECKBOX_TAP_FAILED",
                        "message": e.to_string(),
                        "suggestion": "Try using tap_with_retry or adjusting coordinates"
                    }
                })),
            },
            "tap_switch" => {
                // Switches often need a tap on the right side
                match self.perform_tap(x + 20.0, y) {
                    Ok(_) => Ok(json!({
                        "success": true,
                        "action": "tap_switch",
                        "coordinates": {"x": x + 20.0, "y": y},
                        "device_id": device_id,
                        "message": "Switch tapped with right offset"
                    })),
                    Err(e) => Ok(json!({
                        "success": false,
                        "error": {
                            "code": "SWITCH_TAP_FAILED",
                            "message": e.to_string()
                        }
                    })),
                }
            }
            "double_tap" => match self.perform_double_tap(x, y) {
                Ok(_) => Ok(json!({
                    "success": true,
                    "action": "double_tap",
                    "coordinates": {"x": x, "y": y},
                    "device_id": device_id
                })),
                Err(e) => Ok(json!({
                    "success": false,
                    "error": {
                        "code": "DOUBLE_TAP_FAILED",
                        "message": e.to_string()
                    }
                })),
            },
            "long_press" => match self.perform_long_press(x, y) {
                Ok(_) => Ok(json!({
                    "success": true,
                    "action": "long_press",
                    "coordinates": {"x": x, "y": y},
                    "device_id": device_id
                })),
                Err(e) => Ok(json!({
                    "success": false,
                    "error": {
                        "code": "LONG_PRESS_FAILED",
                        "message": e.to_string()
                    }
                })),
            },
            "tap_with_retry" => {
                let retry_count = params
                    .get("retry_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(3) as usize;

                let mut last_error = None;
                for attempt in 1..=retry_count {
                    match self.perform_tap(x, y) {
                        Ok(_) => {
                            return Ok(json!({
                                "success": true,
                                "action": "tap_with_retry",
                                "coordinates": {"x": x, "y": y},
                                "device_id": device_id,
                                "attempt": attempt,
                                "total_attempts": retry_count
                            }));
                        }
                        Err(e) => {
                            last_error = Some(e);
                            if attempt < retry_count {
                                thread::sleep(Duration::from_millis(500));
                            }
                        }
                    }
                }

                Ok(json!({
                    "success": false,
                    "error": {
                        "code": "TAP_RETRY_FAILED",
                        "message": last_error.map(|e| e.to_string()).unwrap_or_else(|| "Unknown error".to_string()),
                        "attempts": retry_count
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
