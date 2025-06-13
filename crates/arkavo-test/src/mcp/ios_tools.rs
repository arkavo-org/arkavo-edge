use super::device_manager::DeviceManager;
use super::ios_errors::check_ios_availability;
use super::server::{Tool, ToolSchema};
use super::xctest_compiler::XCTestCompiler;
use super::xctest_unix_bridge::{CommandResponse, TapCommand, XCTestUnixBridge};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::process::Command;
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::{Mutex, RwLock};

type XCTestBridgeType = Arc<RwLock<Option<Arc<Mutex<XCTestUnixBridge>>>>>;
static XCTEST_BRIDGE: OnceLock<XCTestBridgeType> = OnceLock::new();

pub struct UiInteractionKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl UiInteractionKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_interaction".to_string(),
                description: "Interact with iOS UI elements. Primary method: idb_companion (embedded binary), with fallback to simctl and AppleScript. 🎯 ALWAYS USE COORDINATES: {\"action\":\"tap\",\"target\":{\"x\":200,\"y\":300}}. The tool automatically initializes IDB companion for reliable interaction. Coordinates should be in logical points for the device.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["tap", "swipe", "type_text", "press_button", "analyze_layout", "clear_text", "select_all", "delete_key", "copy", "paste", "scroll"],
                            "description": "UI interaction type. Use 'analyze_layout' to capture and analyze screen with AI vision for accurate coordinates"
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
                                "text": {"type": "string", "description": "Visible text of element to tap (e.g. button label, link text). XCUITest will search for this text."},
                                "accessibility_id": {"type": "string", "description": "Accessibility identifier of element. More reliable than text as it doesn't change with localization."}
                            }
                        },
                        "value": {
                            "type": "string",
                            "description": "Text to type or button to press (for press_button: 'home', 'power', 'volumeup', 'volumedown')"
                        },
                        "count": {
                            "type": "integer",
                            "description": "Number of times to repeat delete_key action (default: 1)",
                            "minimum": 1
                        },
                        "direction": {
                            "type": "string",
                            "enum": ["up", "down", "left", "right"],
                            "description": "Scroll direction (used with scroll action)"
                        },
                        "amount": {
                            "type": "integer",
                            "description": "Number of scroll steps (used with scroll action, default: 5)",
                            "minimum": 1,
                            "maximum": 50
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

    /// Send a tap command through XCTest bridge (helper to avoid mutex issues)
    async fn send_xctest_tap(
        &self,
        bridge_arc: Arc<Mutex<XCTestUnixBridge>>,
        command: TapCommand,
    ) -> Result<CommandResponse> {
        // Now that we're using tokio::sync::Mutex, we can properly implement this
        let bridge = bridge_arc.lock().await;
        bridge.send_tap_command(command).await
    }

    /// Query actual device dimensions from simulator
    async fn get_device_dimensions(&self, device_id: &str) -> Option<(f64, f64)> {
        // Get device info including device type
        let output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "--json"])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        // Parse JSON to find our device
        let devices_json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;

        // Search through all runtimes for our device
        for (_runtime, device_list) in devices_json["devices"].as_object()? {
            if let Some(devices) = device_list.as_array() {
                for device in devices {
                    if device["udid"].as_str() == Some(device_id) {
                        // Found our device, now get device type details
                        if let Some(device_type_id) = device["deviceTypeIdentifier"].as_str() {
                            // Query device type specifications
                            let devicetype_output = Command::new("xcrun")
                                .args(["simctl", "list", "devicetypes", "--json"])
                                .output()
                                .ok()?;

                            if devicetype_output.status.success() {
                                let types_json: serde_json::Value =
                                    serde_json::from_slice(&devicetype_output.stdout).ok()?;

                                // Look for our device type
                                if let Some(device_types) = types_json["devicetypes"].as_array() {
                                    for dtype in device_types {
                                        if dtype["identifier"].as_str() == Some(device_type_id) {
                                            // Check if screen dimensions are available
                                            if let (Some(width), Some(height)) = (
                                                dtype["screenWidth"].as_f64(),
                                                dtype["screenHeight"].as_f64(),
                                            ) {
                                                eprintln!(
                                                    "Found device dimensions from simctl: {}x{}",
                                                    width, height
                                                );
                                                return Some((width, height));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // If we can't get from simctl, try other methods
        eprintln!("Could not get device dimensions from simctl, using fallback");
        None
    }

    /// Get the XCTest bridge without trying to initialize it
    async fn get_existing_xctest_bridge(&self) -> Option<Arc<Mutex<XCTestUnixBridge>>> {
        let bridge_holder = XCTEST_BRIDGE.get_or_init(|| Arc::new(RwLock::new(None)));
        bridge_holder.read().await.clone()
    }

    /// Get the XCTest bridge (with initialization attempt)
    #[allow(dead_code)]
    async fn get_xctest_bridge(&self) -> Option<Arc<Mutex<XCTestUnixBridge>>> {
        let bridge_holder = XCTEST_BRIDGE.get_or_init(|| Arc::new(RwLock::new(None)));

        // Try to get existing bridge
        let existing = bridge_holder.read().await.clone();
        if existing.is_some() {
            return existing;
        }

        // No bridge exists, try to initialize one
        let mut write_guard = bridge_holder.write().await;

        // Double-check in case another task initialized while we were waiting
        if write_guard.is_some() {
            return write_guard.clone();
        }

        // Try to set up XCTest runner
        match self.setup_xctest_runner().await {
            Ok(bridge) => {
                eprintln!("[UiInteractionKit] XCTest runner initialized successfully");
                let bridge_arc = Arc::new(Mutex::new(bridge));
                *write_guard = Some(bridge_arc.clone());
                Some(bridge_arc)
            }
            Err(e) => {
                eprintln!(
                    "[UiInteractionKit] Failed to initialize XCTest runner: {}. Text-based tapping unavailable.",
                    e
                );
                None
            }
        }
    }

    /// Get the global XCTest bridge storage
    pub fn get_global_xctest_bridge() -> &'static Arc<RwLock<Option<Arc<Mutex<XCTestUnixBridge>>>>>
    {
        XCTEST_BRIDGE.get_or_init(|| Arc::new(RwLock::new(None)))
    }

    /// Set up the XCTest runner
    #[allow(dead_code)]
    async fn setup_xctest_runner(&self) -> Result<XCTestUnixBridge> {
        eprintln!("[UiInteractionKit] Setting up XCTest runner...");

        // Compile XCTest bundle if needed
        let compiler = XCTestCompiler::new()?;
        let socket_path = compiler.socket_path().to_path_buf();
        eprintln!("[UiInteractionKit] Socket path: {}", socket_path.display());

        let bundle_path = compiler.get_xctest_bundle()?;
        eprintln!(
            "[UiInteractionKit] Bundle compiled at: {}",
            bundle_path.display()
        );

        // Get active device
        let device = self
            .device_manager
            .get_active_device()
            .ok_or_else(|| TestError::Mcp("No active device".to_string()))?;
        eprintln!(
            "[UiInteractionKit] Active device: {} ({})",
            device.name, device.id
        );

        // Install to simulator
        eprintln!("[UiInteractionKit] Installing bundle to simulator...");
        compiler.install_to_simulator(&device.id, &bundle_path)?;

        // Create Unix socket bridge with the same socket path
        let mut bridge = XCTestUnixBridge::with_socket_path(socket_path);

        // Start the bridge server BEFORE running tests
        eprintln!("[UiInteractionKit] Starting Unix socket bridge...");
        bridge.start().await?;

        // Give the bridge a moment to start listening
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Launch the test host app - this will connect to our socket
        eprintln!("[UiInteractionKit] Launching test host app...");
        compiler.launch_test_host(&device.id, None)?;

        // Wait for the runner to connect
        eprintln!("[UiInteractionKit] Waiting for test runner to connect...");
        match tokio::time::timeout(
            tokio::time::Duration::from_secs(10),
            bridge.wait_for_connection(),
        )
        .await
        {
            Ok(Ok(())) => {
                eprintln!("[UiInteractionKit] Test runner connected successfully");
                Ok(bridge)
            }
            Ok(Err(e)) => {
                eprintln!("[UiInteractionKit] Connection error: {}", e);
                Err(e)
            }
            Err(_) => {
                eprintln!("[UiInteractionKit] Timeout waiting for test runner connection");
                Err(TestError::Mcp(
                    "Timeout waiting for XCTest runner to connect".to_string(),
                ))
            }
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
            "analyze_layout" => {
                // Capture screenshot and analyze device layout using AI vision
                let device = self
                    .device_manager
                    .get_active_device()
                    .ok_or_else(|| TestError::Mcp("No active device".to_string()))?;
                let device_id = device.id.clone();

                // First capture a screenshot of the entire simulator window
                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string();
                let screenshot_name = format!("layout_analysis_{}.png", timestamp);
                let screenshot_path = format!("test_results/{}", screenshot_name);

                // Ensure test_results directory exists
                std::fs::create_dir_all("test_results").map_err(|e| {
                    TestError::Mcp(format!("Failed to create test_results directory: {}", e))
                })?;

                // Capture the entire simulator window using screencapture
                let window_capture = Command::new("screencapture")
                    .args(["-l", "-o", "-x", &screenshot_path])
                    .arg("$(osascript -e 'tell application \"System Events\" to tell process \"Simulator\" to get id of front window')")
                    .output();

                // If window capture fails, fall back to simulator screenshot
                if window_capture.is_err() || !window_capture.unwrap().status.success() {
                    // Use simctl to capture device screen only
                    let output = Command::new("xcrun")
                        .args(["simctl", "io", &device_id, "screenshot", &screenshot_path])
                        .output()
                        .map_err(|e| {
                            TestError::Mcp(format!("Failed to capture screenshot: {}", e))
                        })?;

                    if !output.status.success() {
                        return Ok(serde_json::json!({
                            "action": "analyze_layout",
                            "success": false,
                            "error": String::from_utf8_lossy(&output.stderr).to_string()
                        }));
                    }
                }

                // Get device info for context
                let device_info = self.device_manager.get_device(&device_id);
                let device_type = device_info
                    .as_ref()
                    .map(|d| d.device_type.as_str())
                    .unwrap_or("unknown");

                // Check if XCUITest is actually available
                let xctest_available = self.get_existing_xctest_bridge().await.is_some();

                let mut response = serde_json::json!({
                    "action": "analyze_layout",
                    "success": true,
                    "screenshot_path": screenshot_path,
                    "device_id": device_id,
                    "device_type": device_type,
                    "instructions": "AI AGENT: The screenshot has been saved. Now use the Read tool to view the image at the path above, then analyze it to identify:\n1. All VISIBLE TEXT on buttons, links, and labels (for text-based tapping)\n2. Text fields and their labels or placeholders\n3. The current screen/view being displayed\n4. Any accessibility hints visible in the UI"
                });

                response["next_steps"] = serde_json::json!(
                    "🎯 USE COORDINATES - The recommended approach:\n1. Read the screenshot image to see UI elements\n2. Identify x,y positions of buttons/fields\n3. Use: {\"action\":\"tap\",\"target\":{\"x\":200,\"y\":300}}\n4. This works immediately without any setup!"
                );
                response["important"] = serde_json::json!(
                    "ALWAYS prefer coordinates over text-based tapping. Coordinates work instantly and are more reliable!"
                );
                if xctest_available {
                    response["xcuitest_status"] = serde_json::json!(
                        "XCUITest is available but NOT RECOMMENDED - use coordinates instead for better reliability"
                    );
                } else {
                    response["xcuitest_status"] = serde_json::json!(
                        "XCUITest not available (and not needed) - coordinates are the primary method"
                    );
                }

                Ok(response)
            }
            "tap" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                // Quick check if XCUITest is available to prevent timeout during lazy init
                let xctest_available = {
                    let bridge_holder = XCTEST_BRIDGE.get_or_init(|| Arc::new(RwLock::new(None)));
                    let guard = bridge_holder.read().await;
                    guard.is_some()
                };

                if let Some(target) = params.get("target") {
                    let mut tap_params = serde_json::json!({});
                    let mut use_xctest = false;
                    let mut xctest_command = None;

                    if let Some(text) = target.get("text").and_then(|v| v.as_str()) {
                        // Only try XCUITest if it's already available
                        if xctest_available {
                            eprintln!("Attempting XCUITest tap by text: {}", text);
                            use_xctest = true;
                            xctest_command = Some(XCTestUnixBridge::create_text_tap(
                                text.to_string(),
                                Some(10.0), // 10 second timeout
                            ));
                        } else {
                            // Return immediate error instead of timing out
                            return Ok(serde_json::json!({
                                "error": {
                                    "code": "USE_COORDINATES_REQUIRED",
                                    "message": format!("Text-based tapping for '{}' is NOT RECOMMENDED! Use coordinate tapping instead - it works immediately without any setup!", text),
                                    "solution": "ALWAYS use coordinates from screenshots instead of text-based tapping",
                                    "required_workflow": [
                                        "1. Use screen_capture to take a screenshot",
                                        "2. Use Read tool to view the screenshot image",
                                        "3. Visually identify the '{}' element position",
                                        "4. Use ui_interaction with coordinates"
                                    ],
                                    "correct_example": {
                                        "action": "tap",
                                        "target": {"x": 200, "y": 400},
                                        "device_id": params.get("device_id").and_then(|v| v.as_str()).unwrap_or("device-id")
                                    },
                                    "why_coordinates_better": [
                                        "✅ Works immediately - no setup required",
                                        "✅ More reliable - no timeouts",
                                        "✅ Faster execution",
                                        "✅ Works with any UI element"
                                    ],
                                    "do_not_use_setup_xcuitest": "setup_xcuitest often fails with timeouts and is not recommended"
                                }
                            }));
                        }
                    } else if let Some(accessibility_id) =
                        target.get("accessibility_id").and_then(|v| v.as_str())
                    {
                        // Only try XCUITest if it's already available
                        if xctest_available {
                            eprintln!(
                                "Attempting XCUITest tap by accessibility ID: {}",
                                accessibility_id
                            );
                            use_xctest = true;
                            xctest_command = Some(XCTestUnixBridge::create_accessibility_tap(
                                accessibility_id.to_string(),
                                Some(10.0), // 10 second timeout
                            ));
                        } else {
                            // Return immediate error instead of timing out
                            return Ok(serde_json::json!({
                                "error": {
                                    "code": "USE_COORDINATES_REQUIRED",
                                    "message": format!("Accessibility ID tapping for '{}' is NOT RECOMMENDED! Use coordinate tapping instead - it works immediately without any setup!", accessibility_id),
                                    "solution": "ALWAYS use coordinates from screenshots instead of accessibility ID tapping",
                                    "required_workflow": [
                                        "1. Use screen_capture to take a screenshot",
                                        "2. Use Read tool to view the screenshot image",
                                        "3. Visually identify the element with accessibility ID '{}'",
                                        "4. Use ui_interaction with coordinates"
                                    ],
                                    "correct_example": {
                                        "action": "tap",
                                        "target": {"x": 200, "y": 400},
                                        "device_id": params.get("device_id").and_then(|v| v.as_str()).unwrap_or("device-id")
                                    },
                                    "why_coordinates_better": [
                                        "✅ Works immediately - no setup required",
                                        "✅ More reliable - no timeouts",
                                        "✅ Faster execution",
                                        "✅ Works with any UI element"
                                    ],
                                    "do_not_use_setup_xcuitest": "setup_xcuitest often fails with timeouts and is not recommended"
                                }
                            }));
                        }
                    } else {
                        // Direct coordinates - check if we should use XCUITest
                        let x = target.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let y = target.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);

                        // Try XCUITest if bridge is available
                        if xctest_available {
                            use_xctest = true;
                            xctest_command = Some(XCTestUnixBridge::create_coordinate_tap(x, y));
                        } else {
                            // Fall back to AppleScript
                            tap_params["x"] = serde_json::json!(x);
                            tap_params["y"] = serde_json::json!(y);
                        }
                    }

                    // If using XCUITest, try to execute the tap
                    if use_xctest && xctest_command.is_some() {
                        if let Some(bridge_arc) = self.get_existing_xctest_bridge().await {
                            let command = xctest_command.unwrap();

                            // Check if connected first
                            let is_connected = {
                                let bridge = bridge_arc.lock().await;
                                bridge.is_connected()
                            };

                            if !is_connected {
                                eprintln!(
                                    "XCUITest bridge not connected, falling back to AppleScript"
                                );
                                if let (Some(x), Some(y)) = (target.get("x"), target.get("y")) {
                                    tap_params["x"] = x.clone();
                                    tap_params["y"] = y.clone();
                                } else {
                                    return Ok(serde_json::json!({
                                        "error": {
                                            "code": "XCUITEST_NOT_CONNECTED",
                                            "message": "XCUITest bridge not connected",
                                            "suggestion": "Use screen_capture to find elements and tap with coordinates instead"
                                        }
                                    }));
                                }
                            } else {
                                // Send the tap command using our helper method
                                match self.send_xctest_tap(bridge_arc, command).await {
                                    Ok(response) => {
                                        if response.success {
                                            return Ok(serde_json::json!({
                                                "success": true,
                                                "action": "tap",
                                                "method": "xcuitest",
                                                "response": response.result,
                                                "device_id": params.get("device_id").and_then(|v| v.as_str()).unwrap_or("active"),
                                                "confidence": "high"
                                            }));
                                        } else {
                                            // XCUITest failed, fall back to AppleScript for coordinates
                                            eprintln!(
                                                "XCUITest tap failed: {:?}, falling back to AppleScript",
                                                response.error
                                            );
                                            if let (Some(x), Some(y)) =
                                                (target.get("x"), target.get("y"))
                                            {
                                                tap_params["x"] = x.clone();
                                                tap_params["y"] = y.clone();
                                            } else {
                                                // Can't fall back for text/accessibility taps
                                                return Ok(serde_json::json!({
                                                    "error": {
                                                        "code": "XCUITEST_TAP_FAILED",
                                                        "message": response.error.unwrap_or_else(|| "XCUITest tap failed".to_string()),
                                                        "suggestion": "For text/accessibility taps, use screen_capture and tap with coordinates instead"
                                                    }
                                                }));
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "XCUITest error: {}, falling back to AppleScript",
                                            e
                                        );
                                        // Fall back to AppleScript for coordinates
                                        if let (Some(x), Some(y)) =
                                            (target.get("x"), target.get("y"))
                                        {
                                            tap_params["x"] = x.clone();
                                            tap_params["y"] = y.clone();
                                        } else {
                                            // Can't fall back for text/accessibility taps
                                            return Ok(serde_json::json!({
                                                "error": {
                                                    "code": "XCUITEST_BRIDGE_ERROR",
                                                    "message": format!("XCUITest bridge error: {}", e),
                                                    "suggestion": "Use screen_capture to find elements and tap with coordinates instead"
                                                }
                                            }));
                                        }
                                    }
                                }
                            }
                        } else {
                            // No XCUITest bridge available
                            if target.get("x").is_none() || target.get("y").is_none() {
                                // Can't proceed without coordinates - provide helpful guidance
                                let text_target = target
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("element");
                                // Get device info for better coordinate guidance
                                let device_id = params
                                    .get("device_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let device_info = self.device_manager.get_device(device_id);
                                let device_type = device_info
                                    .as_ref()
                                    .map(|d| d.device_type.as_str())
                                    .unwrap_or("unknown");

                                // Provide device-specific coordinate tips
                                let (width, height) = match device_type {
                                    s if s.contains("iPhone-16-Pro-Max") => (430, 932),
                                    s if s.contains("iPhone-16-Pro")
                                        || s.contains("iPhone-15-Pro") =>
                                    {
                                        (393, 852)
                                    }
                                    s if s.contains("iPhone-16-Plus")
                                        || s.contains("iPhone-15-Plus") =>
                                    {
                                        (428, 926)
                                    }
                                    s if s.contains("iPhone-16") || s.contains("iPhone-15") => {
                                        (390, 844)
                                    }
                                    s if s.contains("iPhone-SE") => (375, 667),
                                    s if s.contains("iPad") => (820, 1180),
                                    _ => (393, 852), // Default to iPhone Pro size
                                };

                                return Ok(serde_json::json!({
                                    "error": {
                                        "code": "USE_COORDINATES_REQUIRED",
                                        "message": format!("Cannot tap '{}' by text. USE COORDINATES INSTEAD - they work immediately without any setup!", text_target),
                                        "solution": "ALWAYS use coordinates from screenshots. This is the recommended approach!",
                                        "required_workflow": {
                                            "description": "Use coordinates - the PRIMARY and RECOMMENDED method",
                                            "device_info": {
                                                "type": device_type,
                                                "screen_size": format!("{}x{} points", width, height),
                                                "center": {"x": width / 2, "y": height / 2}
                                            },
                                            "steps": [
                                                "1. Use screen_capture to take a screenshot",
                                                "2. Use Read tool to view the screenshot image",
                                                format!("3. Visually locate '{}' in the image", text_target),
                                                "4. Estimate its x,y position",
                                                "5. Use ui_interaction with those coordinates"
                                            ],
                                            "example": {
                                                "action": "tap",
                                                "target": {
                                                    "x": width / 2,
                                                    "y": height / 2,
                                                    "_comment": format!("Replace with actual position of '{}'", text_target)
                                                }
                                            }
                                        },
                                        "why_coordinates_are_better": [
                                            "✅ Works immediately - no setup needed",
                                            "✅ More reliable - no connection timeouts",
                                            "✅ Faster execution",
                                            "✅ Works with embedded idb_companion"
                                        ],
                                        "important": "DO NOT use setup_xcuitest - it often fails with timeouts. Coordinates are the recommended approach!"
                                    }
                                }));
                            }
                        }
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

                    // Try to get actual device dimensions from simulator
                    let (max_x, max_y) = self
                        .get_device_dimensions(&device_id)
                        .await
                        .unwrap_or_else(|| {
                            // Fallback to hardcoded values if query fails
                            match device_type {
                                s if s.contains("iPhone-16-Pro-Max") => (440.0, 956.0),
                                s if s.contains("iPhone-16-Pro") => (402.0, 874.0),
                                s if s.contains("iPhone-16-Plus") => (430.0, 932.0),
                                s if s.contains("iPhone-16") => (393.0, 852.0),
                                s if s.contains("iPhone-15-Pro-Max") => (430.0, 932.0),
                                s if s.contains("iPhone-15-Pro") => (393.0, 852.0),
                                s if s.contains("iPhone-15-Plus") => (428.0, 926.0),
                                s if s.contains("iPhone-15") => (393.0, 852.0),
                                s if s.contains("iPhone-14") => (390.0, 844.0),
                                s if s.contains("iPhone-13") => (390.0, 844.0),
                                s if s.contains("iPhone-SE") => (375.0, 667.0),
                                s if s.contains("iPad") => (1024.0, 1366.0),
                                _ => (390.0, 844.0), // Default to common size
                            }
                        });

                    // Validate and adjust coordinates
                    let adjusted_x = x.min(max_x - 1.0).max(0.0);
                    let adjusted_y = y.min(max_y - 1.0).max(0.0);

                    eprintln!(
                        "UI tap: device={}, type={}, logical_size={}x{}, tap_point=({}, {})",
                        device_id, device_type, max_x, max_y, adjusted_x, adjusted_y
                    );

                    // Special handling for enrollment dialog on iPhone 16 Pro Max
                    if device_type.contains("iPhone-16-Pro-Max")
                        && adjusted_y > 800.0
                        && adjusted_y < 850.0
                    {
                        eprintln!(
                            "Detected possible enrollment dialog tap on iPhone 16 Pro Max - adjusting coordinates"
                        );
                    }

                    // Try multiple methods to ensure tap succeeds
                    #[cfg(target_os = "macos")]
                    {
                        use super::idb_wrapper::IdbWrapper;

                        // Initialize IDB if not already done
                        eprintln!("[ui_interaction] Initializing IDB wrapper...");
                        if let Err(e) = IdbWrapper::initialize() {
                            eprintln!(
                                "[ui_interaction] IDB initialization failed: {}, will try fallback methods",
                                e
                            );
                        } else {
                            // Ensure companion is running for this device
                            eprintln!(
                                "[ui_interaction] Ensuring IDB companion is running for device {}...",
                                device_id
                            );
                            match IdbWrapper::ensure_companion_running(&device_id).await {
                                Ok(_) => {
                                    eprintln!(
                                        "[ui_interaction] IDB companion ready, attempting tap..."
                                    );
                                    // Try idb_companion (embedded binary)
                                    match IdbWrapper::tap(&device_id, adjusted_x, adjusted_y).await
                                    {
                                        Ok(mut result) => {
                                            eprintln!(
                                                "[ui_interaction] IDB tap succeeded at ({}, {})",
                                                adjusted_x, adjusted_y
                                            );
                                            // Add device info to response
                                            if let Some(obj) = result.as_object_mut() {
                                                obj.insert(
                                                    "device_type".to_string(),
                                                    serde_json::json!(device_type),
                                                );
                                                obj.insert(
                                                    "logical_resolution".to_string(),
                                                    serde_json::json!({
                                                        "width": max_x,
                                                        "height": max_y
                                                    }),
                                                );
                                                obj.insert(
                                                    "original_coordinates".to_string(),
                                                    serde_json::json!({
                                                        "x": x,
                                                        "y": y
                                                    }),
                                                );
                                                if x != adjusted_x || y != adjusted_y {
                                                    obj.insert(
                                                        "adjustment_made".to_string(),
                                                        serde_json::json!(true),
                                                    );
                                                    obj.insert("warning".to_string(), serde_json::json!(
                                                        format!("Coordinates were adjusted to fit device bounds. Original: ({}, {}), Adjusted: ({}, {})", 
                                                            x, y, adjusted_x, adjusted_y)
                                                    ));
                                                }
                                            }
                                            return Ok(result);
                                        }
                                        Err(e) => {
                                            eprintln!(
                                                "[ui_interaction] IDB tap failed: {}, trying fallback methods",
                                                e
                                            );
                                            // Continue to fallback methods below
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!(
                                        "[ui_interaction] Failed to ensure companion running: {}, trying fallback methods",
                                        e
                                    );
                                }
                            }
                        }

                        // Method 2: Try simctl io tap (sometimes works when idb fails)
                        let simctl_output = Command::new("xcrun")
                            .args([
                                "simctl",
                                "io",
                                &device_id,
                                "tap",
                                &adjusted_x.to_string(),
                                &adjusted_y.to_string(),
                            ])
                            .output();

                        if let Ok(output) = simctl_output {
                            if output.status.success() {
                                eprintln!(
                                    "UI tap via simctl io succeeded at ({}, {})",
                                    adjusted_x, adjusted_y
                                );
                                return Ok(serde_json::json!({
                                    "success": true,
                                    "action": "tap",
                                    "method": "simctl_io",
                                    "coordinates": {"x": adjusted_x, "y": adjusted_y},
                                    "original_coordinates": {"x": x, "y": y},
                                    "device_id": device_id,
                                    "device_type": device_type,
                                    "logical_resolution": {"width": max_x, "height": max_y},
                                    "confidence": "medium"
                                }));
                            }
                        }

                        // Method 3: Try Accessibility/AppleScript approach
                        let applescript = format!(
                            r#"tell application "Simulator"
                                activate
                                delay 0.1
                            end tell
                            tell application "System Events"
                                tell process "Simulator"
                                    set frontmost to true
                                    click at {{{}, {}}}
                                end tell
                            end tell"#,
                            // Convert logical coordinates to approximate screen coordinates
                            // This is a simple approximation - calibration will help determine exact mapping
                            70.0 + adjusted_x * 1.5, // Rough estimate with bezel offset
                            113.0 + adjusted_y * 1.5  // Rough estimate with title bar + bezel
                        );

                        let applescript_output = Command::new("osascript")
                            .arg("-e")
                            .arg(&applescript)
                            .output();

                        if let Ok(output) = applescript_output {
                            if output.status.success() {
                                eprintln!(
                                    "UI tap via AppleScript/Accessibility succeeded at ({}, {})",
                                    adjusted_x, adjusted_y
                                );
                                return Ok(serde_json::json!({
                                    "success": true,
                                    "action": "tap",
                                    "method": "accessibility_applescript",
                                    "coordinates": {"x": adjusted_x, "y": adjusted_y},
                                    "original_coordinates": {"x": x, "y": y},
                                    "device_id": device_id,
                                    "device_type": device_type,
                                    "logical_resolution": {"width": max_x, "height": max_y},
                                    "confidence": "low",  // Lower confidence until calibrated
                                    "note": "Using rough coordinate mapping - calibration will improve accuracy"
                                }));
                            }
                        }

                        // If all methods fail, report what we tried
                        return Ok(serde_json::json!({
                            "success": false,
                            "action": "tap",
                            "coordinates": {"x": adjusted_x, "y": adjusted_y},
                            "original_coordinates": {"x": x, "y": y},
                            "device_id": device_id,
                            "device_type": device_type,
                            "logical_resolution": {"width": max_x, "height": max_y},
                            "methods_tried": ["idb_companion", "simctl_io", "accessibility_applescript"],
                            "error": {
                                "code": "ALL_METHODS_FAILED",
                                "message": "Unable to perform tap - all methods failed",
                                "suggestion": "Ensure the simulator is responsive and try taking a screenshot to verify the UI state"
                            }
                        }));
                    }

                    #[cfg(not(target_os = "macos"))]
                    {
                        return Ok(serde_json::json!({
                            "success": false,
                            "error": {
                                "code": "PLATFORM_NOT_SUPPORTED",
                                "message": "UI interaction is only supported on macOS",
                                "device_id": device_id
                            }
                        }));
                    }
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

                // Help AI agents who try to use action names as text values
                if text == "clear_text" || text == "delete_key" || text == "select_all" {
                    return Ok(serde_json::json!({
                        "error": {
                            "code": "INCORRECT_ACTION_USAGE",
                            "message": format!("'{}' is an ACTION, not text to type!", text),
                            "correct_usage": {
                                "clear_text": {"action": "clear_text"},
                                "delete_key": {"action": "delete_key", "count": 10},
                                "select_all": {"action": "select_all"}
                            },
                            "example_workflow": [
                                "1. Tap field: {\"action\":\"tap\",\"target\":{\"x\":200,\"y\":400}}",
                                "2. Clear it: {\"action\":\"clear_text\"}",
                                "3. Type text: {\"action\":\"type_text\",\"value\":\"actual text to type\"}"
                            ]
                        }
                    }));
                }

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

                // Try idb_companion type_text first
                #[cfg(target_os = "macos")]
                {
                    use super::idb_wrapper::IdbWrapper;

                    // Initialize IDB if not already done
                    eprintln!("[ui_interaction] Initializing IDB wrapper for type_text...");
                    if let Err(e) = IdbWrapper::initialize() {
                        eprintln!(
                            "[ui_interaction] IDB initialization failed: {}, will try fallback methods",
                            e
                        );
                    } else {
                        // Ensure companion is running for this device
                        eprintln!(
                            "[ui_interaction] Ensuring IDB companion is running for device {}...",
                            device_id
                        );
                        match IdbWrapper::ensure_companion_running(&device_id).await {
                            Ok(_) => {
                                eprintln!(
                                    "[ui_interaction] IDB companion ready, attempting type_text..."
                                );
                                // Try idb_companion type_text
                                match IdbWrapper::type_text(&device_id, text).await {
                                    Ok(result) => {
                                        eprintln!(
                                            "[ui_interaction] IDB type_text succeeded: '{}'",
                                            text
                                        );
                                        return Ok(result);
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "[ui_interaction] IDB type_text failed: {}, trying fallback methods",
                                            e
                                        );
                                        // Continue to fallback methods below
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "[ui_interaction] Failed to ensure companion running: {}, trying fallback methods",
                                    e
                                );
                            }
                        }
                    }
                }

                // Fallback: Type text using AppleScript
                // First ensure the Simulator is active
                let activate_script = r#"tell application "Simulator" to activate"#;
                Command::new("osascript")
                    .arg("-e")
                    .arg(activate_script)
                    .output()
                    .ok();

                // Small delay to ensure focus
                std::thread::sleep(std::time::Duration::from_millis(100));

                // Type the text using AppleScript
                let type_script = format!(
                    r#"tell application "System Events"
                        tell process "Simulator"
                            set frontmost to true
                            keystroke "{}"
                        end tell
                    end tell"#,
                    text.replace("\"", "\\\"").replace("\\", "\\\\")
                );

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(&type_script)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to type text: {}", e)))?;

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "type_text",
                    "text": text,
                    "device_id": device_id,
                    "method": "applescript_keystroke",
                    "error": if !output.status.success() {
                        Some(String::from_utf8_lossy(&output.stderr).to_string())
                    } else {
                        None
                    },
                    "note": "Text typed using AppleScript keystroke simulation.",
                    "ai_hint": "IMPORTANT: You must tap on a text field first to focus it before using type_text. ALWAYS use COORDINATES to tap! Workflow: 1) screen_capture, 2) Read screenshot to find field position, 3) tap using {\"target\":{\"x\":X,\"y\":Y}}, 4) clear_text, 5) type_text. NEVER use text-based tapping - coordinates work better!"
                }))
            }
            "clear_text" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                // Get device ID
                let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
                    id.to_string()
                } else {
                    match self.device_manager.get_active_device() {
                        Some(device) => device.id,
                        None => {
                            return Ok(serde_json::json!({
                                "error": {
                                    "code": "NO_BOOTED_DEVICE",
                                    "message": "No booted iOS device found"
                                }
                            }));
                        }
                    }
                };

                // Clear text by selecting all and deleting
                let clear_script = r#"
                    tell application "System Events"
                        tell process "Simulator"
                            set frontmost to true
                            -- Select all text
                            keystroke "a" using command down
                            delay 0.1
                            -- Delete selected text
                            key code 51
                        end tell
                    end tell
                "#;

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(clear_script)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to clear text: {}", e)))?;

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "clear_text",
                    "device_id": device_id,
                    "method": "select_all_and_delete",
                    "note": "Cleared text field by selecting all (Cmd+A) and deleting"
                }))
            }
            "select_all" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                // Select all text using Cmd+A
                let select_script = r#"
                    tell application "System Events"
                        tell process "Simulator"
                            set frontmost to true
                            keystroke "a" using command down
                        end tell
                    end tell
                "#;

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(select_script)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to select all: {}", e)))?;

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "select_all",
                    "method": "cmd_a"
                }))
            }
            "delete_key" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                // Get repeat count (how many times to press delete)
                let count = params.get("count").and_then(|v| v.as_u64()).unwrap_or(1) as usize;

                // Press delete key using key code 51
                let mut delete_script = r#"
                    tell application "System Events"
                        tell process "Simulator"
                            set frontmost to true"#
                    .to_string();

                for _ in 0..count {
                    delete_script.push_str("\n            key code 51"); // Delete key
                    if count > 1 {
                        delete_script.push_str("\n            delay 0.05");
                    }
                }

                delete_script.push_str(
                    r#"
                        end tell
                    end tell
                "#,
                );

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(&delete_script)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to press delete: {}", e)))?;

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "delete_key",
                    "count": count,
                    "note": format!("Pressed delete key {} time(s)", count)
                }))
            }
            "copy" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                // Copy using Cmd+C
                let copy_script = r#"
                    tell application "System Events"
                        tell process "Simulator"
                            set frontmost to true
                            keystroke "c" using command down
                        end tell
                    end tell
                "#;

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(copy_script)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to copy: {}", e)))?;

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "copy",
                    "method": "cmd_c"
                }))
            }
            "paste" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                // Paste using Cmd+V
                let paste_script = r#"
                    tell application "System Events"
                        tell process "Simulator"
                            set frontmost to true
                            keystroke "v" using command down
                        end tell
                    end tell
                "#;

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(paste_script)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to paste: {}", e)))?;

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "paste",
                    "method": "cmd_v"
                }))
            }
            "scroll" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                // Get direction and amount
                let direction = params
                    .get("direction")
                    .and_then(|v| v.as_str())
                    .unwrap_or("down");

                let amount = params.get("amount").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

                // Map direction to key codes
                let key_code = match direction {
                    "up" => "126",
                    "down" => "125",
                    "left" => "123",
                    "right" => "124",
                    _ => "125", // default to down
                };

                let scroll_script = format!(
                    r#"tell application "Simulator"
                        activate
                    end tell
                    
                    tell application "System Events"
                        tell process "Simulator"
                            set frontmost to true
                            -- Simulate arrow key presses for scrolling
                            repeat {} times
                                key code {}
                                delay 0.05
                            end repeat
                        end tell
                    end tell"#,
                    amount, key_code
                );

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(&scroll_script)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to execute scroll: {}", e)))?;

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "scroll",
                    "direction": direction,
                    "amount": amount,
                    "method": "arrow_keys",
                    "note": "Scrolled using arrow key simulation",
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

                let swipe_data = params.get("swipe");

                // Provide helpful error if swipe parameters are missing or incorrect format
                if swipe_data.is_none() {
                    return Ok(serde_json::json!({
                        "error": {
                            "code": "MISSING_SWIPE_PARAMS",
                            "message": "Missing swipe parameters. Swipe requires a 'swipe' object with coordinates.",
                            "details": {
                                "required_format": {
                                    "action": "swipe",
                                    "swipe": {
                                        "x1": "start x coordinate",
                                        "y1": "start y coordinate",
                                        "x2": "end x coordinate",
                                        "y2": "end y coordinate",
                                        "duration": "optional duration in seconds (default 0.5)"
                                    }
                                },
                                "example": {
                                    "action": "swipe",
                                    "swipe": {
                                        "x1": 215,
                                        "y1": 600,
                                        "x2": 215,
                                        "y2": 200,
                                        "duration": 0.5
                                    }
                                },
                                "note": "Swipe from (x1,y1) to (x2,y2). For scrolling down, use y2 < y1.",
                                "common_swipes": {
                                    "scroll_down": {"x1": 200, "y1": 600, "x2": 200, "y2": 200},
                                    "scroll_up": {"x1": 200, "y1": 200, "x2": 200, "y2": 600},
                                    "swipe_left": {"x1": 300, "y1": 400, "x2": 100, "y2": 400},
                                    "swipe_right": {"x1": 100, "y1": 400, "x2": 300, "y2": 400}
                                },
                                "ai_workflow": "1) Use screen_capture to see current UI, 2) Calculate swipe coordinates based on screen content, 3) Use swipe action with proper coordinates"
                            }
                        }
                    }));
                }

                let swipe_data = swipe_data.unwrap();

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

                // Try idb_companion swipe first
                #[cfg(target_os = "macos")]
                {
                    use super::idb_wrapper::IdbWrapper;

                    // Initialize IDB if not already done
                    eprintln!("[ui_interaction] Initializing IDB wrapper for swipe...");
                    if let Err(e) = IdbWrapper::initialize() {
                        eprintln!(
                            "[ui_interaction] IDB initialization failed: {}, will try fallback methods",
                            e
                        );
                    } else {
                        // Ensure companion is running for this device
                        eprintln!(
                            "[ui_interaction] Ensuring IDB companion is running for device {}...",
                            device_id
                        );
                        match IdbWrapper::ensure_companion_running(&device_id).await {
                            Ok(_) => {
                                eprintln!(
                                    "[ui_interaction] IDB companion ready, attempting swipe..."
                                );
                                // Try idb_companion swipe
                                match IdbWrapper::swipe(&device_id, x1, y1, x2, y2, duration).await
                                {
                                    Ok(result) => {
                                        eprintln!(
                                            "[ui_interaction] IDB swipe succeeded from ({}, {}) to ({}, {})",
                                            x1, y1, x2, y2
                                        );
                                        return Ok(result);
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "[ui_interaction] IDB swipe failed: {}, trying fallback methods",
                                            e
                                        );
                                        // Continue to fallback methods below
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "[ui_interaction] Failed to ensure companion running: {}, trying fallback methods",
                                    e
                                );
                            }
                        }
                    }
                }

                // Fallback: Determine swipe direction and use scroll simulation
                let is_vertical = (x2 - x1).abs() < (y2 - y1).abs();
                let is_scroll_down = y2 < y1; // Swipe up = scroll down
                let is_scroll_right = x2 < x1; // Swipe left = scroll right

                // Use AppleScript with key events for scrolling
                let scroll_script = if is_vertical {
                    let key_code = if is_scroll_down { "125" } else { "126" }; // down: 125, up: 126
                    format!(
                        r#"tell application "Simulator"
                            activate
                        end tell
                        
                        tell application "System Events"
                            tell process "Simulator"
                                set frontmost to true
                                -- Simulate arrow key presses for scrolling
                                repeat 10 times
                                    key code {}
                                    delay 0.05
                                end repeat
                            end tell
                        end tell"#,
                        key_code
                    )
                } else {
                    let key_code = if is_scroll_right { "124" } else { "123" }; // right: 124, left: 123
                    format!(
                        r#"tell application "Simulator"
                            activate
                        end tell
                        
                        tell application "System Events"
                            tell process "Simulator"
                                set frontmost to true
                                -- Simulate arrow key presses for horizontal scrolling
                                repeat 10 times
                                    key code {}
                                    delay 0.05
                                end repeat
                            end tell
                        end tell"#,
                        key_code
                    )
                };

                let output = Command::new("osascript")
                    .arg("-e")
                    .arg(&scroll_script)
                    .output()
                    .map_err(|e| TestError::Mcp(format!("Failed to execute swipe: {}", e)))?;

                Ok(serde_json::json!({
                    "success": output.status.success(),
                    "action": "swipe",
                    "method": "scroll_simulation",
                    "direction": if is_vertical {
                        if is_scroll_down { "scroll_down" } else { "scroll_up" }
                    } else if is_scroll_right { "scroll_right" } else { "scroll_left" },
                    "coordinates": {
                        "x1": x1, "y1": y1,
                        "x2": x2, "y2": y2
                    },
                    "duration": duration,
                    "device_id": device_id,
                    "note": "Swipe simulated using arrow keys. For better scrolling, use the 'scroll' action instead.",
                    "suggestion": "Use {\"action\":\"scroll\",\"direction\":\"down\",\"amount\":10} for more reliable scrolling",
                    "error": if !output.status.success() {
                        Some(String::from_utf8_lossy(&output.stderr).to_string())
                    } else {
                        None
                    }
                }))
            }
            "press_button" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                let button = params
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing button value".to_string()))?;

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

                // Try idb_companion press_button first
                #[cfg(target_os = "macos")]
                {
                    use super::idb_wrapper::IdbWrapper;

                    // Initialize IDB if not already done
                    eprintln!("[ui_interaction] Initializing IDB wrapper for press_button...");
                    if let Err(e) = IdbWrapper::initialize() {
                        eprintln!("[ui_interaction] IDB initialization failed: {}", e);
                        return Ok(serde_json::json!({
                            "success": false,
                            "action": "press_button",
                            "button": button,
                            "device_id": device_id,
                            "error": {
                                "code": "IDB_INIT_FAILED",
                                "message": format!("IDB initialization failed: {}", e),
                                "supported_buttons": ["home", "power", "volumeup", "volumedown"],
                                "note": "Hardware button simulation requires idb_companion"
                            }
                        }));
                    } else {
                        // Ensure companion is running for this device
                        eprintln!(
                            "[ui_interaction] Ensuring IDB companion is running for device {}...",
                            device_id
                        );
                        match IdbWrapper::ensure_companion_running(&device_id).await {
                            Ok(_) => {
                                eprintln!(
                                    "[ui_interaction] IDB companion ready, attempting press_button..."
                                );
                                // Try idb_companion press_button
                                match IdbWrapper::press_button(&device_id, button).await {
                                    Ok(result) => {
                                        eprintln!(
                                            "[ui_interaction] IDB press_button succeeded: '{}'",
                                            button
                                        );
                                        return Ok(result);
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "[ui_interaction] IDB press_button failed: {}, no fallback available",
                                            e
                                        );
                                        return Ok(serde_json::json!({
                                            "success": false,
                                            "action": "press_button",
                                            "button": button,
                                            "device_id": device_id,
                                            "error": {
                                                "code": "BUTTON_PRESS_FAILED",
                                                "message": format!("Failed to press button '{}': {}", button, e),
                                                "supported_buttons": ["home", "power", "volumeup", "volumedown"],
                                                "note": "Hardware button simulation requires idb_companion"
                                            }
                                        }));
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "[ui_interaction] Failed to ensure companion running: {}",
                                    e
                                );
                                return Ok(serde_json::json!({
                                    "success": false,
                                    "action": "press_button",
                                    "button": button,
                                    "device_id": device_id,
                                    "error": {
                                        "code": "IDB_NOT_AVAILABLE",
                                        "message": format!("IDB companion not available: {}", e),
                                        "supported_buttons": ["home", "power", "volumeup", "volumedown"],
                                        "note": "Hardware button simulation requires idb_companion"
                                    }
                                }));
                            }
                        }
                    }
                }

                #[cfg(not(target_os = "macos"))]
                {
                    return Ok(serde_json::json!({
                        "success": false,
                        "error": {
                            "code": "PLATFORM_NOT_SUPPORTED",
                            "message": "Press button is only supported on macOS",
                            "device_id": device_id
                        }
                    }));
                }
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
                description: "Capture iOS simulator screen. 🎯 COORDINATE WORKFLOW (RECOMMENDED): 1) Use screen_capture to take screenshot, 2) Read the image file to see UI, 3) Identify element positions visually, 4) ALWAYS use ui_interaction with coordinates {\"target\":{\"x\":X,\"y\":Y}}. ⚠️ AVOID text-based tapping - it requires setup_xcuitest which often fails! Example: If you see a 'Sign In' button at position (200,400), use {\"action\":\"tap\",\"target\":{\"x\":200,\"y\":400}} NOT text-based tapping.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Name for the screenshot (optional - will generate timestamp-based name if not provided)"
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
                    "required": []
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
            .unwrap_or_else(|| {
                // Generate random name with timestamp if not provided
                let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%3f");
                let random_suffix: String = (0..4)
                    .map(|_| {
                        use rand::Rng;
                        let n = rand::thread_rng().gen_range(0..36);
                        if n < 10 {
                            (b'0' + n) as char
                        } else {
                            (b'a' + n - 10) as char
                        }
                    })
                    .collect();
                Box::leak(format!("screenshot_{}_{}", timestamp, random_suffix).into_boxed_str())
            });

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
                description: "Query UI elements (LIMITED). AI AGENTS: This tool has limited functionality without XCTest. Instead, use this workflow: 1) screen_capture to get screenshot, 2) Read the image file, 3) Use your vision capabilities to identify UI elements and coordinates, 4) Use tap/swipe/type_text with those coordinates.".to_string(),
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

        // Since we can't access UI elements directly without XCTest,
        // provide guidance on alternative approaches
        match query_type {
            "accessibility_tree" => Ok(serde_json::json!({
                "device_id": device_id,
                "status": "not_available",
                "alternative": "Use screen_capture tool to take a screenshot, then analyze visually",
                "note": "Direct accessibility tree access requires XCTest. Consider using coordinate-based interactions with known UI layouts."
            })),
            "visible_elements" => {
                // For AI agents, suggest using screenshot analysis instead
                Ok(serde_json::json!({
                    "device_id": device_id,
                    "elements": [],
                    "status": "limited_support",
                    "alternatives": {
                        "method1": "Use screen_capture to take screenshot and analyze the image",
                        "method2": "Use known coordinates for common UI elements based on app design",
                        "method3": "Try tapping at different coordinates and observe results"
                    },
                    "note": "Without XCTest, element detection is not available. AI agents should use visual analysis of screenshots."
                }))
            }
            "text_content" => Ok(serde_json::json!({
                "device_id": device_id,
                "texts": [],
                "status": "not_available",
                "alternative": "Use screen_capture tool and analyze text in the screenshot image",
                "note": "Direct text extraction requires XCTest. Screenshots can be analyzed for text content."
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xctest_tap_command_creation() {
        // Test that we can create tap commands
        let coord_tap = XCTestUnixBridge::create_coordinate_tap(100.0, 200.0);
        assert_eq!(coord_tap.parameters.x, Some(100.0));
        assert_eq!(coord_tap.parameters.y, Some(200.0));
        assert!(coord_tap.parameters.text.is_none());
        assert!(coord_tap.parameters.accessibility_id.is_none());

        let text_tap = XCTestUnixBridge::create_text_tap("Login".to_string(), Some(5.0));
        assert_eq!(text_tap.parameters.text, Some("Login".to_string()));
        assert_eq!(text_tap.parameters.timeout, Some(5.0));
        assert!(text_tap.parameters.x.is_none());
        assert!(text_tap.parameters.y.is_none());

        let acc_tap = XCTestUnixBridge::create_accessibility_tap("login_button".to_string(), None);
        assert_eq!(
            acc_tap.parameters.accessibility_id,
            Some("login_button".to_string())
        );
        assert!(acc_tap.parameters.timeout.is_none());
    }
}
