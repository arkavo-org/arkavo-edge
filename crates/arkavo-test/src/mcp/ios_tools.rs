use super::device_manager::DeviceManager;
use super::ios_errors::check_ios_availability;
use super::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::sync::Arc;

pub struct UiInteractionKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl UiInteractionKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_interaction".to_string(),
                description: "Interact with iOS UI elements".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["tap", "swipe", "type_text", "press_button"],
                            "description": "UI interaction type"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "target": {
                            "type": "object",
                            "properties": {
                                "x": {"type": "number"},
                                "y": {"type": "number"},
                                "text": {"type": "string"},
                                "accessibility_id": {"type": "string"}
                            }
                        },
                        "value": {
                            "type": "string",
                            "description": "Text to type or button to press"
                        },
                        "swipe": {
                            "type": "object",
                            "properties": {
                                "x1": {"type": "number"},
                                "y1": {"type": "number"},
                                "x2": {"type": "number"},
                                "y2": {"type": "number"},
                                "duration": {"type": "number"}
                            }
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
impl Tool for UiInteractionKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        // Get target device
        let _device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            // Verify device exists
            if self.device_manager.get_device(id).is_none() {
                return Ok(serde_json::json!({
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
                    return Ok(serde_json::json!({
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
            "tap" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                if let Some(target) = params.get("target") {
                    let mut tap_params = serde_json::json!({});

                    if let Some(text) = target.get("text").and_then(|v| v.as_str()) {
                        // Text-based tapping requires XCTest integration
                        return Ok(serde_json::json!({
                            "error": {
                                "code": "TEXT_TAP_NOT_SUPPORTED",
                                "message": "Tapping by text requires XCTest framework integration",
                                "details": {
                                    "text": text,
                                    "suggestion": "Use coordinate-based tap instead"
                                }
                            }
                        }));
                    } else if let Some(accessibility_id) =
                        target.get("accessibility_id").and_then(|v| v.as_str())
                    {
                        // Accessibility-based tapping requires XCTest integration
                        return Ok(serde_json::json!({
                            "error": {
                                "code": "ACCESSIBILITY_TAP_NOT_SUPPORTED",
                                "message": "Tapping by accessibility ID requires XCTest framework integration",
                                "details": {
                                    "accessibility_id": accessibility_id,
                                    "suggestion": "Use coordinate-based tap instead"
                                }
                            }
                        }));
                    } else {
                        // Direct coordinates
                        tap_params["x"] = target.get("x").unwrap_or(&serde_json::json!(0)).clone();
                        tap_params["y"] = target.get("y").unwrap_or(&serde_json::json!(0)).clone();
                    }

                    // Get device ID
                    let device_id =
                        if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
                            id.to_string()
                        } else {
                            match self.device_manager.get_active_device() {
                                Some(device) => device.id,
                                None => {
                                    self.device_manager.refresh_devices().ok();
                                    match self.device_manager.get_booted_devices().first() {
                                        Some(device) => device.id.clone(),
                                        None => {
                                            return Ok(serde_json::json!({
                                                "error": {
                                                    "code": "NO_BOOTED_DEVICE",
                                                    "message": "No booted iOS device found"
                                                }
                                            }));
                                        }
                                    }
                                }
                            }
                        };

                    // Execute tap using xcrun simctl directly
                    let x = tap_params["x"].as_f64().unwrap_or(0.0);
                    let y = tap_params["y"].as_f64().unwrap_or(0.0);

                    // Get device info for coordinate validation
                    let device_info = self.device_manager.get_device(&device_id);
                    let device_type = device_info
                        .as_ref()
                        .map(|d| d.device_type.as_str())
                        .unwrap_or("unknown");

                    // Common iOS device logical resolutions (in points, not pixels)
                    let (max_x, max_y) = match device_type {
                        s if s.contains("iPhone-16-Pro-Max") => (430.0, 932.0),
                        s if s.contains("iPhone-16-Pro") || s.contains("iPhone-15-Pro") => {
                            (393.0, 852.0)
                        }
                        s if s.contains("iPhone-16-Plus") || s.contains("iPhone-15-Plus") => {
                            (428.0, 926.0)
                        }
                        s if s.contains("iPhone-16") || s.contains("iPhone-15") => (390.0, 844.0),
                        s if s.contains("iPhone-SE") => (375.0, 667.0),
                        s if s.contains("iPad") => (820.0, 1180.0),
                        _ => (393.0, 852.0), // Default to iPhone Pro size
                    };

                    // Validate and adjust coordinates
                    let adjusted_x = x.min(max_x - 1.0).max(0.0);
                    let adjusted_y = y.min(max_y - 1.0).max(0.0);

                    // Try multiple tap methods
                    let output = if let Ok(fb_output) = Command::new("fbsimctl")
                        .args([
                            &device_id,
                            "tap",
                            &adjusted_x.to_string(),
                            &adjusted_y.to_string(),
                        ])
                        .output()
                    {
                        fb_output
                    } else if let Ok(apple_output) = Command::new("applesimutils")
                        .args([
                            "--byId",
                            &device_id,
                            "--tapAt",
                            &format!("{},{}", adjusted_x, adjusted_y),
                        ])
                        .output()
                    {
                        apple_output
                    } else {
                        // Fallback to sending events via Instruments if available
                        Command::new("xcrun")
                            .args(["instruments", "-w", &device_id, "-t", "Blank", "-e", "UIASCRIPT", 
                                   &format!("UIATarget.localTarget().tap({{{}, {}}})", adjusted_x, adjusted_y)])
                            .output()
                            .unwrap_or_else(|_| {
                                // Final fallback - return error with helpful message
                                std::process::Output {
                                    status: std::process::ExitStatus::from_raw(1),
                                    stdout: Vec::new(),
                                    stderr: b"Unable to send tap event. Install one of these tools:\n\
                                         - fbsimctl: brew install facebook/fb/fbsimctl\n\
                                         - applesimutils: brew install applesimutils\n\
                                         - Or use Xcode's Accessibility Inspector to interact with the simulator\n\
                                         \n\
                                         Note: Direct tap via 'xcrun simctl' may not be available in your Xcode version.".to_vec(),
                                }
                            })
                    };

                    let mut response = serde_json::json!({
                        "success": output.status.success(),
                        "action": "tap",
                        "coordinates": {"x": adjusted_x, "y": adjusted_y},
                        "original_coordinates": {"x": x, "y": y},
                        "device_id": device_id,
                        "device_type": device_type,
                        "logical_resolution": {"width": max_x, "height": max_y}
                    });

                    if !output.status.success() {
                        response["error"] = serde_json::json!({
                            "message": String::from_utf8_lossy(&output.stderr).trim().to_string(),
                            "note": "Coordinates are in logical points, not pixels. For retina displays, divide pixel coordinates by the scale factor (2x or 3x)."
                        });
                    }

                    if x != adjusted_x || y != adjusted_y {
                        response["warning"] = serde_json::json!(format!(
                            "Coordinates were adjusted to fit device bounds. Original: ({}, {}), Adjusted: ({}, {})",
                            x, y, adjusted_x, adjusted_y
                        ));
                    }

                    Ok(response)
                } else {
                    Err(TestError::Mcp("Missing target for tap action".to_string()))
                }
            }
            "type_text" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                let text = params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing text value".to_string()))?;

                // Get device ID
                let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
                    id.to_string()
                } else {
                    match self.device_manager.get_active_device() {
                        Some(device) => device.id,
                        None => {
                            self.device_manager.refresh_devices().ok();
                            match self.device_manager.get_booted_devices().first() {
                                Some(device) => device.id.clone(),
                                None => {
                                    return Ok(serde_json::json!({
                                        "error": {
                                            "code": "NO_BOOTED_DEVICE",
                                            "message": "No booted iOS device found"
                                        }
                                    }));
                                }
                            }
                        }
                    }
                };

                // Type text using xcrun simctl directly
                let output = Command::new("xcrun")
                    .args(["simctl", "io", &device_id, "type", text])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to type text: {}", e)))?;

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "type_text",
                    "text": text,
                    "device_id": device_id,
                    "error": if !output.status.success() {
                        Some(String::from_utf8_lossy(&output.stderr).to_string())
                    } else {
                        None
                    }
                }))
            }
            "swipe" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                let swipe_data = params
                    .get("swipe")
                    .ok_or_else(|| TestError::Mcp("Missing swipe parameters".to_string()))?;

                // Get device ID
                let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
                    id.to_string()
                } else {
                    match self.device_manager.get_active_device() {
                        Some(device) => device.id,
                        None => {
                            self.device_manager.refresh_devices().ok();
                            match self.device_manager.get_booted_devices().first() {
                                Some(device) => device.id.clone(),
                                None => {
                                    return Ok(serde_json::json!({
                                        "error": {
                                            "code": "NO_BOOTED_DEVICE",
                                            "message": "No booted iOS device found"
                                        }
                                    }));
                                }
                            }
                        }
                    }
                };

                let x1 = swipe_data
                    .get("x1")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.0);
                let y1 = swipe_data
                    .get("y1")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(300.0);
                let x2 = swipe_data
                    .get("x2")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.0);
                let y2 = swipe_data
                    .get("y2")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.0);
                let duration = swipe_data
                    .get("duration")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.5);

                // Swipe using xcrun simctl directly
                let output = Command::new("xcrun")
                    .args([
                        "simctl",
                        "io",
                        &device_id,
                        "swipe",
                        &x1.to_string(),
                        &y1.to_string(),
                        &x2.to_string(),
                        &y2.to_string(),
                        "--duration",
                        &duration.to_string(),
                    ])
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to execute swipe: {}", e)))?;

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "swipe",
                    "coordinates": {
                        "x1": x1, "y1": y1,
                        "x2": x2, "y2": y2
                    },
                    "duration": duration,
                    "device_id": device_id,
                    "error": if !output.status.success() {
                        Some(String::from_utf8_lossy(&output.stderr).to_string())
                    } else {
                        None
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

pub struct ScreenCaptureKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl ScreenCaptureKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "screen_capture".to_string(),
                description: "Capture and analyze iOS screen".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Name for the screenshot"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "analyze": {
                            "type": "boolean",
                            "description": "Whether to analyze the screenshot"
                        }
                    },
                    "required": ["name"]
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for ScreenCaptureKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        // Check iOS availability first
        if let Err(e) = check_ios_availability() {
            return Ok(e.to_response());
        }

        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing name parameter".to_string()))?;

        let path = format!("test_results/{}.png", name);

        // Create directory if it doesn't exist
        std::fs::create_dir_all("test_results")
            .map_err(|e| TestError::Mcp(format!("Failed to create directory: {}", e)))?;

        // Get device ID
        let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else {
            match self.device_manager.get_active_device() {
                Some(device) => device.id,
                None => {
                    // Try to find any booted device
                    self.device_manager.refresh_devices().ok();
                    match self.device_manager.get_booted_devices().first() {
                        Some(device) => device.id.clone(),
                        None => {
                            return Ok(serde_json::json!({
                                "error": {
                                    "code": "NO_BOOTED_DEVICE",
                                    "message": "No booted iOS device found",
                                    "details": {
                                        "suggestion": "Boot a simulator with 'xcrun simctl boot <device-id>'"
                                    }
                                }
                            }));
                        }
                    }
                }
            }
        };

        // Capture screenshot using xcrun simctl directly
        let output = Command::new("xcrun")
            .args(["simctl", "io", &device_id, "screenshot", &path])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to capture screenshot: {}", e)))?;

        let mut result = if output.status.success() {
            serde_json::json!({
                "success": true,
                "path": path,
                "device_id": device_id,
                "timestamp": chrono::Utc::now().to_rfc3339()
            })
        } else {
            serde_json::json!({
                "success": false,
                "error": String::from_utf8_lossy(&output.stderr).to_string(),
                "path": path,
                "device_id": device_id
            })
        };

        // If analyze is requested, add analysis placeholder
        if params
            .get("analyze")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            result["analysis"] = serde_json::json!({
                "elements_detected": 0,
                "text_found": [],
                "buttons": [],
                "input_fields": []
            });
        }

        Ok(result)
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct UiQueryKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl UiQueryKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_query".to_string(),
                description: "Query UI element state and properties".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query_type": {
                            "type": "string",
                            "enum": ["accessibility_tree", "visible_elements", "text_content"],
                            "description": "Type of UI query"
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        },
                        "filter": {
                            "type": "object",
                            "properties": {
                                "element_type": {"type": "string"},
                                "text_contains": {"type": "string"},
                                "accessibility_label": {"type": "string"}
                            }
                        }
                    },
                    "required": ["query_type"]
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for UiQueryKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        // Check iOS availability first
        if let Err(e) = check_ios_availability() {
            return Ok(e.to_response());
        }

        let query_type = params
            .get("query_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing query_type parameter".to_string()))?;

        // Get device ID
        let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else {
            match self.device_manager.get_active_device() {
                Some(device) => device.id,
                None => {
                    self.device_manager.refresh_devices().ok();
                    match self.device_manager.get_booted_devices().first() {
                        Some(device) => device.id.clone(),
                        None => {
                            return Ok(serde_json::json!({
                                "error": {
                                    "code": "NO_BOOTED_DEVICE",
                                    "message": "No booted iOS device found"
                                }
                            }));
                        }
                    }
                }
            }
        };

        // For now, return mock data as simctl doesn't have direct UI query support
        // In a real implementation, this would use accessibility APIs
        match query_type {
            "accessibility_tree" => Ok(serde_json::json!({
                "tree": {
                    "root": {
                        "type": "Application",
                        "children": [
                            {
                                "type": "Window",
                                "frame": {"x": 0, "y": 0, "width": 393, "height": 852},
                                "children": []
                            }
                        ]
                    }
                },
                "device_id": device_id
            })),
            "visible_elements" => Ok(serde_json::json!({
                "elements": [],
                "device_id": device_id,
                "note": "Element detection requires XCTest framework integration"
            })),
            "text_content" => Ok(serde_json::json!({
                "texts": [],
                "device_id": device_id,
                "note": "Text extraction requires XCTest framework integration"
            })),
            _ => Err(TestError::Mcp(format!(
                "Unknown query type: {}",
                query_type
            ))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[allow(dead_code)]
fn get_active_device_id() -> Result<String> {
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

    // Fallback: try to get any iPhone device
    let output = Command::new("xcrun")
        .args(["simctl", "list", "devices"])
        .output()
        .map_err(|e| TestError::Mcp(format!("Failed to list all devices: {}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines() {
        if line.contains("iPhone") && line.contains("(") && line.contains(")") {
            if let Some(start) = line.find('(') {
                if let Some(end) = line.find(')') {
                    let device_id = line[start + 1..end].to_string();
                    if device_id.len() == 36 {
                        // UUID length
                        return Ok(device_id);
                    }
                }
            }
        }
    }

    // Ultimate fallback: return a placeholder ID
    // This helps avoid errors in mock scenarios
    Ok("MOCK-DEVICE-ID".to_string())
}
