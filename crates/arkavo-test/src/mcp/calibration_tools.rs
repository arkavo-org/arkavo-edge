use super::server::{Tool, ToolSchema};
use super::calibration::server::{CalibrationServer, CalibrationRequest, CalibrationResponse};
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::path::PathBuf;
use std::process::Command;

pub struct CalibrationTool {
    schema: ToolSchema,
    server: Arc<CalibrationServer>,
}

impl CalibrationTool {
    pub fn new() -> Result<Self> {
        let storage_path = PathBuf::from("/tmp/arkavo_calibration");
        let server = CalibrationServer::new(storage_path)
            .map_err(|e| TestError::Mcp(e.to_string()))?;
        
        Ok(Self {
            schema: ToolSchema {
                name: "calibration_manager".to_string(),
                description: "Manages UI automation calibration for iOS simulators. Automatically calibrates coordinate systems and interaction patterns using a reference app.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "description": "Action to perform: install_reference_app, start_calibration, get_status, get_calibration, list_devices, enable_monitoring, export_calibration, import_calibration",
                            "enum": ["install_reference_app", "start_calibration", "get_status", "get_calibration", "list_devices", "enable_monitoring", "export_calibration", "import_calibration"]
                        },
                        "device_id": {
                            "type": "string",
                            "description": "Device ID for calibration operations"
                        },
                        "session_id": {
                            "type": "string",
                            "description": "Session ID for status checks"
                        },
                        "bundle_id": {
                            "type": "string",
                            "description": "Optional bundle ID of the reference app. Defaults to com.arkavo.reference"
                        },
                        "enabled": {
                            "type": "boolean",
                            "description": "Enable/disable auto monitoring"
                        },
                        "calibration_data": {
                            "type": "string",
                            "description": "Calibration data for import"
                        }
                    },
                    "required": ["action"]
                }),
            },
            server: Arc::new(server),
        })
    }
}

#[async_trait]
impl Tool for CalibrationTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        eprintln!("[CalibrationTool] Execute called with params: {:?}", params);
        let action = params["action"].as_str()
            .ok_or_else(|| TestError::Mcp("action is required".to_string()))?;
        eprintln!("[CalibrationTool] Action: {}", action);
        
        match action {
            "start_calibration" => {
                let device_id = params["device_id"].as_str()
                    .ok_or_else(|| TestError::Mcp("device_id is required for start_calibration".to_string()))?;
                let bundle_id = params["bundle_id"].as_str().map(|s| s.to_string());
                
                // Check if ArkavoReference app is installed
                let app_check = Command::new("xcrun")
                    .args(["simctl", "get_app_container", device_id, "com.arkavo.reference"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                
                if !app_check {
                    // Automatically install the reference app
                    eprintln!("[CalibrationTool] ArkavoReference app not found. Installing automatically...");
                    
                    let install_result = self.build_and_install_reference_app(device_id).await?;
                    
                    // Check if installation was successful
                    if let Some(success) = install_result.get("success").and_then(|v| v.as_bool()) {
                        if !success {
                            return Ok(json!({
                                "success": false,
                                "error": {
                                    "code": "REFERENCE_APP_INSTALL_FAILED",
                                    "message": "Failed to install ArkavoReference app",
                                    "details": install_result.get("error").cloned().unwrap_or(json!("Unknown error")),
                                    "critical": true
                                }
                            }));
                        }
                    }
                    
                    eprintln!("[CalibrationTool] ArkavoReference app installed successfully. Proceeding with calibration.");
                }
                
                let request = CalibrationRequest::StartCalibration {
                    device_id: device_id.to_string(),
                    reference_bundle_id: bundle_id,
                };
                
                match self.server.handle_request(request).await {
                    CalibrationResponse::SessionStarted { session_id } => {
                        Ok(json!({
                            "success": true,
                            "message": format!("Calibration started successfully with ArkavoReference app. Session ID: {}", session_id),
                            "session_id": session_id,
                            "device_id": device_id,
                            "status": "started",
                            "calibration_process": {
                                "description": "Automated calibration process for iOS UI coordinate mapping",
                                "phases": [
                                    {
                                        "phase": 1,
                                        "name": "Initialization",
                                        "duration": "3-5 seconds",
                                        "description": "Launch ArkavoReference app and wait for calibration view to load"
                                    },
                                    {
                                        "phase": 2,
                                        "name": "Tap Sequence",
                                        "duration": "10-15 seconds", 
                                        "description": "Perform 5 calibration taps at percentage-based grid points: 20%/20%, 80%/20%, 50%/50%, 20%/80%, 80%/80%"
                                    },
                                    {
                                        "phase": 3,
                                        "name": "Verification",
                                        "duration": "5 seconds",
                                        "description": "Read tap results from app's Documents folder and calculate coordinate offsets"
                                    },
                                    {
                                        "phase": 4,
                                        "name": "Validation",
                                        "duration": "2-3 seconds",
                                        "description": "Store calibration data and mark session complete"
                                    }
                                ],
                                "total_duration": "20-30 seconds",
                                "retry_policy": "Up to 3 attempts if offset detection fails"
                            },
                            "technical_details": {
                                "tap_method": "idb_companion with --only simulator flag",
                                "verification_method": "File-based communication via app Documents directory",
                                "expected_files": [
                                    "{app_container}/Documents/calibration_results.json"
                                ],
                                "coordinate_system": "Logical coordinates (device-specific, e.g. 402x874 for iPhone 16 Pro)"
                            },
                            "next_steps": [
                                "Monitor calibration progress using 'get_status' action with the session_id",
                                "Status progression: initializing -> validating -> complete",
                                "Once complete, use 'get_calibration' to retrieve calibration data"
                            ],
                            "troubleshooting": {
                                "stuck_in_initializing": "App may not be detecting taps. Auto-recovery will be attempted after 15 seconds",
                                "no_verification_data": "App may not be writing results. Check Documents folder permissions",
                                "calibration_timeout": "Process times out after 60 seconds",
                                "idb_stuck": "If no taps are detected for 15 seconds, automatic IDB recovery will be triggered"
                            },
                            "auto_recovery": {
                                "enabled": true,
                                "watchdog_timeout": "15 seconds",
                                "tap_timeout": "10 seconds per tap",
                                "description": "Calibration manager will automatically recover from stuck IDB operations"
                            }
                        }))
                    }
                    CalibrationResponse::Error { message } => {
                        Err(TestError::Mcp(message))
                    }
                    _ => Err(TestError::Mcp("Unexpected response".to_string()))
                }
            }
            
            "get_status" => {
                let session_id = params["session_id"].as_str()
                    .ok_or_else(|| TestError::Mcp("session_id is required for get_status".to_string()))?;
                
                let request = CalibrationRequest::GetStatus {
                    session_id: session_id.to_string(),
                };
                
                match self.server.handle_request(request).await {
                    CalibrationResponse::Status(status) => {
                        let is_complete = status.status == "complete" || status.status == "failed";
                        let mut response = json!({
                            "success": true,
                            "status": status.status,
                            "session_id": status.session_id,
                            "device_id": status.device_id,
                            "start_time": status.start_time,
                            "elapsed_seconds": status.elapsed_seconds,
                            "tap_count": status.tap_count,
                            "last_tap_time": status.last_tap_time,
                            "idb_status": {
                                "connected": status.idb_status.connected,
                                "companion_running": status.idb_status.companion_running,
                                "last_health_check": status.idb_status.last_health_check,
                                "last_error": status.idb_status.last_error
                            }
                        });
                        
                        // Check if ArkavoReference app is running by checking launchctl
                        let app_running = Command::new("xcrun")
                            .args(["simctl", "spawn", &status.device_id, "launchctl", "list"])
                            .output()
                            .ok()
                            .and_then(|output| {
                                if output.status.success() {
                                    let output_str = String::from_utf8_lossy(&output.stdout);
                                    // Look for the app in launchctl output (includes PID and other info)
                                    let is_running = output_str.lines()
                                        .any(|line| line.contains("UIKitApplication:com.arkavo.reference"));
                                    eprintln!("[CalibrationTool] App running check for device {}: {}", status.device_id, is_running);
                                    Some(is_running)
                                } else {
                                    eprintln!("[CalibrationTool] Failed to check app status via launchctl");
                                    None
                                }
                            })
                            .unwrap_or(false);
                        
                        response["app_running"] = json!(app_running);
                        
                        // Check for IDB issues
                        let idb_warning = if !status.idb_status.connected || !status.idb_status.companion_running {
                            // Check if it's a framework issue
                            if let Some(error) = &status.idb_status.last_error {
                                if error.contains("Library not loaded") && error.contains("FBControlCore") {
                                    Some("IDB companion has missing framework dependencies. Use 'idb_management' tool with 'install' action to fix.")
                                } else if error.contains("Connection refused") || error.contains("not connected") {
                                    Some("IDB companion is not connected to the device. The MCP server will handle auto-recovery.")
                                } else if error.contains("timeout") || error.contains("timed out") {
                                    Some("IDB operation timed out. Auto-recovery will restart IDB companion.")
                                } else {
                                    Some("IDB companion encountered an error. Check idb_status.last_error for details.")
                                }
                            } else if !status.idb_status.companion_running {
                                Some("IDB companion process is not running. Use 'idb_management' tool with 'recover' action.")
                            } else {
                                Some("IDB companion is not connected. Auto-recovery will attempt to fix this.")
                            }
                        } else {
                            None
                        };
                        
                        if let Some(warning) = idb_warning {
                            response["idb_warning"] = json!(warning);
                            
                            // Add specific action if framework issue
                            if status.idb_status.last_error.as_ref().map(|e| e.contains("FBControlCore")).unwrap_or(false) {
                                response["recommended_action"] = json!({
                                    "tool": "idb_management",
                                    "action": "install",
                                    "description": "Install IDB with proper framework dependencies"
                                });
                            }
                        }
                        
                        // Check for stuck calibration (no taps in last 10 seconds during validating phase)
                        if status.status == "validating" && status.tap_count < 5 {
                            if let Some(last_tap) = status.last_tap_time {
                                let seconds_since_tap = (chrono::Utc::now() - last_tap).num_seconds();
                                if seconds_since_tap > 10 {
                                    response["stuck_warning"] = json!(format!(
                                        "No taps detected in {} seconds. Auto-recovery will trigger at 15 seconds.",
                                        seconds_since_tap
                                    ));
                                    response["auto_recovery_status"] = json!({
                                        "will_trigger_in": (15 - seconds_since_tap).max(0),
                                        "description": "Automatic IDB recovery will attempt to fix the issue"
                                    });
                                }
                            } else if status.elapsed_seconds > 10 {
                                response["stuck_warning"] = json!("No taps detected. Auto-recovery will trigger soon.");
                                response["auto_recovery_status"] = json!({
                                    "will_trigger_in": (15 - status.elapsed_seconds as i64).max(0),
                                    "description": "Automatic IDB recovery will attempt to fix the issue"
                                });
                            }
                        }
                        
                        // Add guidance based on status
                        match status.status.as_str() {
                            "initializing" => {
                                if !app_running {
                                    response["message"] = json!("Calibration Phase 1: App launch failed");
                                    response["phase"] = json!({
                                        "current": 1,
                                        "name": "Initialization",
                                        "status": "failed",
                                        "description": "ArkavoReference app is not running"
                                    });
                                    response["next_action"] = json!("Check if the app is installed and try launching it manually");
                                    response["troubleshooting"] = json!([
                                        "Verify the app is installed: xcrun simctl get_app_container <device_id> com.arkavo.reference",
                                        "Try launching manually: xcrun simctl launch <device_id> com.arkavo.reference",
                                        "Check device logs for launch errors"
                                    ]);
                                } else {
                                    let elapsed = status.elapsed_seconds;
                                    response["message"] = json!(format!("Calibration Phase 1: Initialization ({}s elapsed)", elapsed));
                                    response["phase"] = json!({
                                        "current": 1,
                                        "name": "Initialization", 
                                        "status": "in_progress",
                                        "description": "App launched, waiting for calibration view",
                                        "expected_duration": "3-5 seconds",
                                        "warning": if elapsed > 10 { "Taking longer than expected" } else { "" }
                                    });
                                    response["next_action"] = json!("Wait a few seconds and check status again");
                                }
                            }
                            "validating" => {
                                let elapsed = status.elapsed_seconds;
                                
                                // Check if we're in auto-recovery mode
                                let is_recovering = status.tap_count == 0 && elapsed > 15;
                                
                                if is_recovering {
                                    response["message"] = json!("Calibration Phase 2: Auto-recovery in progress");
                                    response["phase"] = json!({
                                        "current": 2,
                                        "name": "Auto-Recovery",
                                        "status": "recovering",
                                        "description": "Automatic IDB recovery triggered due to no tap progress",
                                        "recovery_steps": [
                                            "Terminating stuck IDB companion processes",
                                            "Clearing IDB cache",
                                            "Re-initializing IDB connection",
                                            "Retrying tap sequence"
                                        ],
                                        "elapsed": elapsed
                                    });
                                    response["next_action"] = json!("Wait for auto-recovery to complete (usually 5-10 seconds)");
                                } else {
                                    response["message"] = json!(format!("Calibration Phase 2-3: Tap sequence and verification ({}s elapsed, {} taps completed)", elapsed, status.tap_count));
                                    response["phase"] = json!({
                                        "current": 2,
                                        "name": "Tap Sequence & Verification",
                                        "status": "in_progress", 
                                        "description": "Performing calibration taps and reading results",
                                        "expected_duration": "15-20 seconds",
                                        "tap_percentages": ["20%/20%", "80%/20%", "50%/50%", "20%/80%", "80%/80%"],
                                        "progress": format!("{}/5 taps completed", status.tap_count),
                                        "note": "Actual pixel coordinates calculated based on device screen size"
                                    });
                                    response["next_action"] = json!("Continue checking status until complete");
                                }
                            }
                            "complete" => {
                                response["message"] = json!("Calibration completed successfully!");
                                response["phase"] = json!({
                                    "current": 4,
                                    "name": "Complete",
                                    "status": "success",
                                    "description": "Calibration data stored and ready for use"
                                });
                                response["next_action"] = json!("Use 'get_calibration' action to retrieve the calibration data");
                            }
                            "failed" => {
                                response["message"] = json!("Calibration failed. Check logs for details.");
                                response["phase"] = json!({
                                    "current": 0,
                                    "name": "Failed", 
                                    "status": "error",
                                    "description": status.status.clone()
                                });
                                response["next_action"] = json!("Review error details and retry calibration");
                            }
                            _ => {}
                        }
                        
                        response["is_complete"] = json!(is_complete);
                        Ok(response)
                    }
                    CalibrationResponse::Error { message } => {
                        Err(TestError::Mcp(message))
                    }
                    _ => Err(TestError::Mcp("Unexpected response".to_string()))
                }
            }
            
            "get_calibration" => {
                let device_id = params["device_id"].as_str()
                    .ok_or_else(|| TestError::Mcp("device_id is required for get_calibration".to_string()))?;
                
                match self.server.data_store.get_calibration(device_id) {
                    Some(config) => {
                        let result = self.server.data_store.get_latest_result(device_id)
                            .map_err(|e| TestError::Mcp(e.to_string()))?;
                        
                        Ok(json!({
                            "success": true,
                            "message": format!("Calibration found for device {}", device_id),
                            "config": config,
                            "result": result,
                            "is_valid": self.server.data_store.is_calibration_valid(device_id, 24 * 7)
                        }))
                    }
                    None => {
                        Ok(json!({
                            "success": false,
                            "status": "not_found",
                            "message": format!("No calibration found for device {}", device_id),
                            "device_id": device_id,
                            "suggestion": "Start a new calibration using the 'start_calibration' action",
                            "note": "Calibration may still be in progress. Check status with 'get_status' if you have a session_id."
                        }))
                    }
                }
            }
            
            "list_devices" => {
                eprintln!("[CalibrationTool] Handling list_devices action");
                let devices = self.server.data_store.list_calibrated_devices();
                eprintln!("[CalibrationTool] Found {} devices", devices.len());
                
                let response = json!({
                    "success": true,
                    "message": format!("Found {} calibrated devices", devices.len()),
                    "devices": devices,
                    "count": devices.len()
                });
                eprintln!("[CalibrationTool] Returning response: {:?}", response);
                Ok(response)
            }
            
            "enable_monitoring" => {
                let enabled = params["enabled"].as_bool()
                    .ok_or_else(|| TestError::Mcp("enabled is required for enable_monitoring".to_string()))?;
                
                self.server.enable_auto_monitoring(enabled).await;
                
                Ok(json!({
                    "success": true,
                    "message": format!("Auto-monitoring {}", if enabled { "enabled" } else { "disabled" }),
                    "monitoring_enabled": enabled
                }))
            }
            
            "export_calibration" => {
                let device_id = params["device_id"].as_str()
                    .ok_or_else(|| TestError::Mcp("device_id is required for export_calibration".to_string()))?;
                
                let export_data = self.server.data_store.export_calibration(device_id)
                    .map_err(|e| TestError::Mcp(e.to_string()))?;
                
                Ok(json!({
                    "success": true,
                    "message": "Calibration exported successfully",
                    "export_data": export_data,
                    "device_id": device_id
                }))
            }
            
            "import_calibration" => {
                let calibration_data = params["calibration_data"].as_str()
                    .ok_or_else(|| TestError::Mcp("calibration_data is required for import_calibration".to_string()))?;
                
                let device_id = self.server.data_store.import_calibration(calibration_data)
                    .map_err(|e| TestError::Mcp(e.to_string()))?;
                
                Ok(json!({
                    "success": true,
                    "message": format!("Calibration imported successfully for device {}", device_id),
                    "device_id": device_id,
                    "status": "imported"
                }))
            }
            
            "install_reference_app" => {
                let device_id = params["device_id"].as_str()
                    .ok_or_else(|| TestError::Mcp("device_id is required for install_reference_app".to_string()))?;
                
                // Build and install the ArkavoReference app
                self.build_and_install_reference_app(device_id).await
            }
            
            _ => Err(TestError::Mcp(format!("Unknown action: {}", action)))
        }
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

impl CalibrationTool {
    async fn build_and_install_reference_app(&self, device_id: &str) -> Result<Value> {
        eprintln!("[CalibrationTool] Building and installing ArkavoReference app...");
        
        // Get the path to the iOS project
        let ios_project_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("ios")
            .join("ArkavoReference");
            
        if !ios_project_path.exists() {
            return Ok(json!({
                "success": false,
                "error": {
                    "code": "PROJECT_NOT_FOUND",
                    "message": "ArkavoReference iOS project not found",
                    "expected_path": ios_project_path.display().to_string(),
                    "hint": "The ArkavoReference project should be in ios/ArkavoReference/"
                }
            }));
        }
        
        // Build the app using xcodebuild
        eprintln!("[CalibrationTool] Building ArkavoReference app...");
        let build_output = Command::new("xcodebuild")
            .args([
                "-project", "ArkavoReference.xcodeproj",
                "-scheme", "ArkavoReference",
                "-configuration", "Debug",
                "-sdk", "iphonesimulator",
                "-derivedDataPath", "build",
                "build"
            ])
            .current_dir(&ios_project_path)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run xcodebuild: {}", e)))?;
            
        if !build_output.status.success() {
            let error_msg = String::from_utf8_lossy(&build_output.stderr);
            eprintln!("[CalibrationTool] Build failed: {}", error_msg);
            
            return Ok(json!({
                "success": false,
                "error": {
                    "code": "BUILD_FAILED",
                    "message": "Failed to build ArkavoReference app",
                    "details": error_msg.to_string(),
                    "hint": "Make sure Xcode is installed and the project builds correctly"
                }
            }));
        }
        
        // Find the built app
        let app_path = ios_project_path
            .join("build")
            .join("Build")
            .join("Products")
            .join("Debug-iphonesimulator")
            .join("ArkavoReference.app");
            
        if !app_path.exists() {
            // Try alternative path
            let alt_path = ios_project_path
                .join("build")
                .join("Build")
                .join("Products")
                .join("Debug-iphonesimulator")
                .join("ArkavoReferenceApp.app");
                
            if !alt_path.exists() {
                return Ok(json!({
                    "success": false,
                    "error": {
                        "code": "APP_NOT_FOUND",
                        "message": "Built app not found at expected location",
                        "searched_paths": [
                            app_path.display().to_string(),
                            alt_path.display().to_string()
                        ]
                    }
                }));
            }
        }
        
        // Install to simulator
        eprintln!("[CalibrationTool] Installing app to simulator {}...", device_id);
        let install_output = Command::new("xcrun")
            .args([
                "simctl",
                "install",
                device_id,
                app_path.to_str().unwrap()
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to install app: {}", e)))?;
            
        if !install_output.status.success() {
            let error_msg = String::from_utf8_lossy(&install_output.stderr);
            return Ok(json!({
                "success": false,
                "error": {
                    "code": "INSTALL_FAILED",
                    "message": "Failed to install ArkavoReference app",
                    "details": error_msg.to_string(),
                    "device_id": device_id,
                    "hint": "Make sure the simulator is booted"
                }
            }));
        }
        
        eprintln!("[CalibrationTool] Successfully installed ArkavoReference app");
        
        Ok(json!({
            "success": true,
            "message": "ArkavoReference app built and installed successfully",
            "device_id": device_id,
            "bundle_id": "com.arkavo.reference",
            "app_path": app_path.display().to_string(),
            "next_steps": [
                "Use calibration_manager with action 'start_calibration' to begin calibration",
                "Or use deep_link tool to open arkavo-edge://calibration"
            ]
        }))
    }
}

pub struct CalibrationStatusTool {
    schema: ToolSchema,
    server: Arc<CalibrationServer>,
}

impl CalibrationStatusTool {
    pub fn new(server: Arc<CalibrationServer>) -> Self {
        Self {
            schema: ToolSchema {
                name: "calibration_status".to_string(),
                description: "Quick status check for all calibrations and recommendations".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID to check specific device"
                        }
                    }
                }),
            },
            server,
        }
    }
}

#[async_trait]
impl Tool for CalibrationStatusTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        if let Some(device_id) = params["device_id"].as_str() {
            // Check specific device
            if let Some(config) = self.server.data_store.get_calibration(device_id) {
                let is_valid = self.server.data_store.is_calibration_valid(device_id, 24 * 7);
                let age_hours = (chrono::Utc::now() - config.last_calibrated).num_hours();
                
                Ok(json!({
                    "success": true,
                    "message": format!(
                        "Device {} calibration: {} (age: {} hours)",
                        device_id,
                        if is_valid { "VALID" } else { "NEEDS RECALIBRATION" },
                        age_hours
                    ),
                    "device_id": device_id,
                    "is_valid": is_valid,
                    "age_hours": age_hours,
                    "last_calibrated": config.last_calibrated
                }))
            } else {
                Ok(json!({
                    "success": true,
                    "message": format!("Device {} has never been calibrated", device_id),
                    "device_id": device_id,
                    "calibrated": false
                }))
            }
        } else {
            // Check all devices
            let devices = self.server.data_store.list_calibrated_devices();
            let valid_count = devices.iter().filter(|d| d.is_valid).count();
            let invalid_count = devices.len() - valid_count;
            
            Ok(json!({
                "success": true,
                "message": format!(
                    "Calibration summary: {} valid, {} need recalibration, {} total",
                    valid_count, invalid_count, devices.len()
                ),
                "total_devices": devices.len(),
                "valid_count": valid_count,
                "invalid_count": invalid_count,
                "devices": devices
            }))
        }
    }
    
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}