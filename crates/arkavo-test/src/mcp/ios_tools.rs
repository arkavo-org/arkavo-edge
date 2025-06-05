use super::device_manager::DeviceManager;
use super::ios_errors::check_ios_availability;
use super::server::{Tool, ToolSchema};
use super::xctest_compiler::XCTestCompiler;
use super::xctest_unix_bridge::{CommandResponse, TapCommand, XCTestUnixBridge};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::{Mutex, OnceCell};

static XCTEST_BRIDGE: OnceCell<Option<Arc<Mutex<XCTestUnixBridge>>>> = OnceCell::const_new();

pub struct UiInteractionKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl UiInteractionKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "ui_interaction".to_string(),
                description: "Interact with iOS UI elements using XCUITest (when available) or fallback methods. SUPPORTS: 1) TEXT-BASED TAPS: {\"action\":\"tap\",\"target\":{\"text\":\"Login\"}} - finds and taps elements by visible text. 2) ACCESSIBILITY ID: {\"action\":\"tap\",\"target\":{\"accessibility_id\":\"login_button\"}} - taps by accessibility identifier. 3) COORDINATES: {\"action\":\"tap\",\"target\":{\"x\":200,\"y\":300}} - direct coordinate tap. BEST PRACTICE: Use text/accessibility_id when possible as they're more reliable than coordinates. TEXT INPUT: 1) Tap field first, 2) Use clear_text if needed, 3) type_text. XCUITest provides 10-second timeout for finding elements.".to_string(),
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
                            "description": "Text to type or button to press"
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

    /// Get or initialize the XCTest bridge
    async fn get_xctest_bridge(&self) -> Option<Arc<Mutex<XCTestUnixBridge>>> {
        XCTEST_BRIDGE
            .get_or_init(|| async {
                // Try to compile and set up XCTest runner
                match self.setup_xctest_runner().await {
                    Ok(bridge) => {
                        eprintln!("XCTest runner initialized successfully");
                        Some(Arc::new(Mutex::new(bridge)))
                    }
                    Err(e) => {
                        eprintln!(
                            "Failed to initialize XCTest runner: {}. Falling back to AppleScript.",
                            e
                        );
                        None
                    }
                }
            })
            .await
            .clone()
    }

    /// Set up the XCTest runner
    async fn setup_xctest_runner(&self) -> Result<XCTestUnixBridge> {
        eprintln!("[UiInteractionKit] Setting up XCTest runner...");
        
        // Compile XCTest bundle if needed
        let compiler = XCTestCompiler::new()?;
        let socket_path = compiler.socket_path().to_path_buf();
        eprintln!("[UiInteractionKit] Socket path: {}", socket_path.display());
        
        let bundle_path = compiler.get_xctest_bundle()?;
        eprintln!("[UiInteractionKit] Bundle compiled at: {}", bundle_path.display());

        // Get active device
        let device = self
            .device_manager
            .get_active_device()
            .ok_or_else(|| TestError::Mcp("No active device".to_string()))?;
        eprintln!("[UiInteractionKit] Active device: {} ({})", device.name, device.id);

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

        // Run the test bundle - this will connect to our socket
        eprintln!("[UiInteractionKit] Running test bundle...");
        compiler.run_tests(&device.id, "com.arkavo.testrunner")?;

        // Wait for the runner to connect
        eprintln!("[UiInteractionKit] Waiting for test runner to connect...");
        match tokio::time::timeout(
            tokio::time::Duration::from_secs(10),
            bridge.wait_for_connection()
        ).await {
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
                Err(TestError::Mcp("Timeout waiting for XCTest runner to connect".to_string()))
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
                let xctest_available = self.get_xctest_bridge().await.is_some();
                
                let mut response = serde_json::json!({
                    "action": "analyze_layout",
                    "success": true,
                    "screenshot_path": screenshot_path,
                    "device_id": device_id,
                    "device_type": device_type,
                    "instructions": "AI AGENT: The screenshot has been saved. Now use the Read tool to view the image at the path above, then analyze it to identify:\n1. All VISIBLE TEXT on buttons, links, and labels (for text-based tapping)\n2. Text fields and their labels or placeholders\n3. The current screen/view being displayed\n4. Any accessibility hints visible in the UI"
                });
                
                if xctest_available {
                    response["next_steps"] = serde_json::json!("After reading the image, use text-based interactions:\n- Buttons: {\"action\":\"tap\",\"target\":{\"text\":\"Sign In\"}}\n- Text fields: Tap using nearby label text\n- XCUITest will find elements by text with 10-second timeout");
                    response["xcuitest_status"] = serde_json::json!("XCUITest is available for text-based element finding");
                } else {
                    response["next_steps"] = serde_json::json!("After reading the image, you'll need to use coordinates:\n1. Estimate the x,y position of elements from the image\n2. Use: {\"action\":\"tap\",\"target\":{\"x\":200,\"y\":300}}\n3. Text-based tapping requires XCUITest which is not currently available");
                    response["xcuitest_status"] = serde_json::json!("XCUITest not available - use coordinates from image analysis");
                }
                
                Ok(response)
            }
            "tap" => {
                // Check iOS availability first
                if let Err(e) = check_ios_availability() {
                    return Ok(e.to_response());
                }

                if let Some(target) = params.get("target") {
                    let mut tap_params = serde_json::json!({});
                    let mut use_xctest = false;
                    let mut xctest_command = None;

                    if let Some(text) = target.get("text").and_then(|v| v.as_str()) {
                        // Try XCUITest for text-based tapping
                        eprintln!("Attempting XCUITest tap by text: {}", text);
                        use_xctest = true;
                        xctest_command = Some(XCTestUnixBridge::create_text_tap(
                            text.to_string(),
                            Some(10.0), // 10 second timeout
                        ));
                    } else if let Some(accessibility_id) =
                        target.get("accessibility_id").and_then(|v| v.as_str())
                    {
                        // Try XCUITest for accessibility ID tapping
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
                        // Direct coordinates - check if we should use XCUITest
                        let x = target.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let y = target.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0);

                        // Try XCUITest if bridge is available
                        if let Some(_bridge) = self.get_xctest_bridge().await {
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
                        if let Some(bridge_arc) = self.get_xctest_bridge().await {
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
                                                "device_id": params.get("device_id").and_then(|v| v.as_str()).unwrap_or("active")
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
                                let text_target = target.get("text").and_then(|v| v.as_str()).unwrap_or("element");
                                return Ok(serde_json::json!({
                                    "error": {
                                        "code": "XCUITEST_NOT_AVAILABLE",
                                        "message": format!("Cannot tap '{}' - XCUITest not available and no coordinates provided", text_target),
                                        "details": "Text-based element finding requires XCUITest which failed to initialize. This is normal in some environments.",
                                        "suggestion": "Use coordinate-based tapping instead:",
                                        "steps": [
                                            "1. If you haven't already, use analyze_layout to capture screenshot",
                                            "2. Use the Read tool to view the screenshot", 
                                            "3. Visually locate the element in the image",
                                            "4. Estimate its x,y coordinates",
                                            "5. Use ui_interaction with those coordinates"
                                        ],
                                        "example": {
                                            "analyze_layout": {},
                                            "read": {"file_path": "test_results/layout_analysis_[timestamp].png"},
                                            "ui_interaction": {"action": "tap", "target": {"x": 200, "y": 300}}
                                        },
                                        "coordinate_tips": {
                                            "iPhone_16_Pro_Max": "Screen is 430x932 points",
                                            "center": {"x": 215, "y": 466},
                                            "typical_button_height": 44
                                        }
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

                    // Use AppleScript with Accessibility API to find the actual device screen
                    let applescript = format!(
                        r#"tell application "Simulator"
                            activate
                            delay 0.1
                            tell application "System Events"
                                tell process "Simulator"
                                    set frontmost to true
                                    
                                    try
                                        -- Try to find the device screen using Accessibility
                                        -- The device screen is typically an AXGroup within the window
                                        set deviceScreen to missing value
                                        set uiElements to UI elements of front window
                                        
                                        repeat with elem in uiElements
                                            if role of elem is "AXGroup" then
                                                set deviceScreen to elem
                                                exit repeat
                                            end if
                                        end repeat
                                        
                                        if deviceScreen is not missing value then
                                            -- Found the device screen element
                                            set {{screenX, screenY}} to position of deviceScreen
                                            set {{screenWidth, screenHeight}} to size of deviceScreen
                                            
                                            -- Map logical coordinates to screen coordinates
                                            set clickX to screenX + ({} * screenWidth / {})
                                            set clickY to screenY + ({} * screenHeight / {})
                                        else
                                            -- Fallback: couldn't find device screen, use window with estimates
                                            set simWindow to front window
                                            set {{windowX, windowY}} to position of simWindow
                                            set {{windowWidth, windowHeight}} to size of simWindow
                                            
                                            -- Default estimates (will vary by device)
                                            set titleBarHeight to 28
                                            set bezelSize to 20
                                            
                                            -- Calculate estimated content area
                                            set contentX to windowX + bezelSize
                                            set contentY to windowY + titleBarHeight + bezelSize
                                            set contentWidth to windowWidth - (bezelSize * 2)
                                            set contentHeight to windowHeight - titleBarHeight - (bezelSize * 2)
                                            
                                            set clickX to contentX + ({} * contentWidth / {})
                                            set clickY to contentY + ({} * contentHeight / {})
                                        end if
                                        
                                        -- Perform the click
                                        click at {{clickX, clickY}}
                                        
                                    on error errMsg
                                        -- If all else fails, try a simple click based on window position
                                        set simWindow to front window
                                        set {{windowX, windowY}} to position of simWindow
                                        set {{windowWidth, windowHeight}} to size of simWindow
                                        
                                        -- Very rough estimate
                                        set clickX to windowX + 30 + ({} * (windowWidth - 60) / {})
                                        set clickY to windowY + 50 + ({} * (windowHeight - 80) / {})
                                        
                                        click at {{clickX, clickY}}
                                    end try
                                end tell
                            end tell
                        end tell"#,
                        adjusted_x,
                        max_x,
                        adjusted_y,
                        max_y,
                        adjusted_x,
                        max_x,
                        adjusted_y,
                        max_y,
                        adjusted_x,
                        max_x,
                        adjusted_y,
                        max_y
                    );

                    let output = Command::new("osascript")
                        .arg("-e")
                        .arg(&applescript)
                        .output()
                        .unwrap_or_else(|e| std::process::Output {
                            status: std::process::ExitStatus::from_raw(1),
                            stdout: Vec::new(),
                            stderr: format!("Failed to execute tap via AppleScript: {}", e)
                                .into_bytes(),
                        });

                    let mut response = serde_json::json!({
                        "success": output.status.success(),
                        "action": "tap",
                        "coordinates": {"x": adjusted_x, "y": adjusted_y},
                        "original_coordinates": {"x": x, "y": y},
                        "device_id": device_id,
                        "device_type": device_type,
                        "logical_resolution": {"width": max_x, "height": max_y},
                        "tap_method": "accessibility_applescript"
                    });

                    if !output.status.success() {
                        let error_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();
                        response["error"] = serde_json::json!({
                            "message": error_msg,
                            "fallback_action": "If tap fails, try: 1) Ensure Simulator is frontmost app, 2) Use screen_capture to verify UI state, 3) Adjust coordinates if needed",
                            "coordinates_note": "Coordinates are in logical points relative to the simulator screen content."
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

                // Type text using AppleScript
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
                    "ai_hint": "IMPORTANT: You must tap on a text field first to focus it before using type_text. To clear existing text, use clear_text action or select_all followed by delete_key. Workflow: 1) screen_capture, 2) tap on text field, 3) clear_text, 4) type_text. DO NOT use idb commands directly - use MCP tools instead."
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

                // Determine swipe direction and use scroll instead
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
                description: "Capture iOS simulator screen. WORKFLOW FOR AI AGENTS: 1) Use screen_capture to take screenshot, 2) Read the image file to see UI, 3) PREFER using ui_interaction with text/accessibility_id when you can see button labels or UI text (more reliable than coordinates), 4) Only use coordinates as last resort. The screenshot helps you identify text labels for XCUITest to find. Example: If you see a 'Sign In' button, use {\"action\":\"tap\",\"target\":{\"text\":\"Sign In\"}} rather than coordinates.".to_string(),
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
