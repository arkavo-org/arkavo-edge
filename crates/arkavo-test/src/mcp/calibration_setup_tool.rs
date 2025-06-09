use super::device_manager::DeviceManager;
use super::server::{Tool, ToolSchema};
use crate::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::process::Command;

pub struct CalibrationSetupKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl CalibrationSetupKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "setup_calibration".to_string(),
                description: "Setup and launch the test host app in calibration mode. The app will display tap coordinates prominently on screen for easy screenshot detection. Each tap by the MCP server will show X,Y coordinates in large text.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Optional device ID. If not specified, uses active device."
                        }
                    }
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for CalibrationSetupKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        eprintln!("[CalibrationSetupKit] Starting calibration setup...");

        // Get device ID
        let device_id = if let Some(id) = params.get("device_id").and_then(|v| v.as_str()) {
            id.to_string()
        } else {
            match self.device_manager.get_active_device() {
                Some(device) => device.id,
                None => {
                    return Ok(serde_json::json!({
                        "success": false,
                        "error": {
                            "code": "NO_ACTIVE_DEVICE",
                            "message": "No active device set. Use device_management tool first."
                        }
                    }));
                }
            }
        };

        // Check if ArkavoReference app is installed
        let check_output = Command::new("xcrun")
            .args(["simctl", "get_app_container", &device_id, "com.arkavo.ArkavoReference"])
            .output()?;

        if !check_output.status.success() {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "REFERENCE_APP_NOT_INSTALLED",
                    "message": "ArkavoReference app is not installed. Please install the ArkavoReference app first."
                }
            }));
        }

        // Terminate app if already running
        eprintln!("[CalibrationSetupKit] Terminating any existing reference app...");
        let _ = Command::new("xcrun")
            .args(["simctl", "terminate", &device_id, "com.arkavo.ArkavoReference"])
            .output();
        
        // Launch the app first
        eprintln!("[CalibrationSetupKit] Launching ArkavoReference app...");
        let launch_output = Command::new("xcrun")
            .args(["simctl", "launch", &device_id, "com.arkavo.ArkavoReference"])
            .output()?;

        if !launch_output.status.success() {
            return Ok(serde_json::json!({
                "success": false,
                "error": {
                    "code": "LAUNCH_FAILED",
                    "message": format!("Failed to launch ArkavoReference app: {}", 
                        String::from_utf8_lossy(&launch_output.stderr))
                }
            }));
        }

        // Wait for app to load
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        
        // Navigate to calibration mode using deep link
        eprintln!("[CalibrationSetupKit] Opening calibration mode via deep link...");
        let deeplink_output = Command::new("xcrun")
            .args(["simctl", "openurl", &device_id, "arkavo-edge://calibration"])
            .output()?;
            
        if !deeplink_output.status.success() {
            eprintln!("[CalibrationSetupKit] Warning: Failed to open calibration deep link: {}", 
                String::from_utf8_lossy(&deeplink_output.stderr));
        }
        
        // Handle URL confirmation dialog
        eprintln!("[CalibrationSetupKit] Handling URL confirmation dialog...");
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        
        // Tap "Open" button (centered on most iPhone models)
        let tap_output = Command::new("xcrun")
            .args(["simctl", "io", &device_id, "tap", "195", "490"])
            .output()?;
            
        if !tap_output.status.success() {
            eprintln!("[CalibrationSetupKit] Warning: Failed to tap URL dialog, it may not have appeared");
        } else {
            eprintln!("[CalibrationSetupKit] Successfully handled URL confirmation dialog");
        }

        // Wait for app to load
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        Ok(serde_json::json!({
            "success": true,
            "status": "calibration_ready",
            "message": "Calibration mode is now active. The app will display tap coordinates on screen.",
            "device_id": device_id,
            "features": {
                "coordinate_display": "Large text showing X,Y coordinates for each tap",
                "visual_markers": "Green circles mark tap locations",
                "coordinate_labels": "Small labels show coordinates at each tap point",
                "real_time_feedback": "Coordinates update immediately on tap",
                "screenshot_friendly": "High contrast display for easy OCR/detection"
            },
            "instructions": [
                "The app is now waiting for taps",
                "Each tap will display coordinates in large text: 'X: 123 Y: 456'",
                "Take screenshots after each tap to capture the coordinates",
                "Tap markers remain on screen to show all tap locations",
                "After 5 taps, calibration will complete automatically"
            ],
            "next_steps": [
                "Use ui_interaction tool to tap at various screen locations",
                "Take screenshots to capture the displayed coordinates",
                "Compare expected vs actual coordinates for calibration"
            ]
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}