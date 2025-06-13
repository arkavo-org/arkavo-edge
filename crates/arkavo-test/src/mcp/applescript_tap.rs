use serde_json::json;
use std::process::Command;

use crate::{Result, TestError};

/// Direct AppleScript-based tap implementation for macOS
/// This bypasses IDB and uses the Accessibility API directly
pub struct AppleScriptTap;

impl AppleScriptTap {
    /// Perform a tap using AppleScript and System Events
    pub async fn tap(device_id: &str, x: f64, y: f64) -> Result<serde_json::Value> {
        eprintln!(
            "[AppleScriptTap] Performing tap at ({}, {}) for device {}",
            x, y, device_id
        );

        // First, get the simulator window position
        let window_bounds = Self::get_simulator_window_bounds(device_id).await?;

        // Calculate absolute screen coordinates
        let screen_x = window_bounds.0 + x;
        let screen_y = window_bounds.1 + y;

        eprintln!(
            "[AppleScriptTap] Window at ({}, {}), tapping at screen ({}, {})",
            window_bounds.0, window_bounds.1, screen_x, screen_y
        );

        // Focus the Simulator app
        let focus_script = r#"
        tell application "Simulator"
            activate
        end tell
        delay 0.1
        "#;

        let _ = Command::new("osascript")
            .arg("-e")
            .arg(focus_script)
            .output();

        // Perform the tap
        let tap_script = format!(
            r#"
            tell application "System Events"
                click at {{{}, {}}}
            end tell
            "#,
            screen_x as i32, screen_y as i32
        );

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&tap_script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute AppleScript: {}", e)))?;

        if output.status.success() {
            eprintln!("[AppleScriptTap] Tap succeeded");
            Ok(json!({
                "success": true,
                "method": "applescript_direct",
                "action": "tap",
                "coordinates": {"x": x, "y": y},
                "screen_coordinates": {"x": screen_x, "y": screen_y},
                "device_id": device_id,
                "confidence": "high"
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("[AppleScriptTap] Tap failed: {}", stderr);
            Err(TestError::Mcp(format!(
                "AppleScript tap failed: {}",
                stderr
            )))
        }
    }

    /// Get simulator window bounds for coordinate conversion
    async fn get_simulator_window_bounds(device_id: &str) -> Result<(f64, f64, f64, f64)> {
        // Get the device name from simctl
        let device_name = Self::get_device_name(device_id).await?;

        let script = format!(
            r#"
            tell application "System Events"
                tell process "Simulator"
                    set allWindows to windows
                    repeat with aWindow in allWindows
                        set windowName to name of aWindow
                        if windowName contains "{}" then
                            set windowPosition to position of aWindow
                            set windowSize to size of aWindow
                            return (item 1 of windowPosition & "," & item 2 of windowPosition & "," & item 1 of windowSize & "," & item 2 of windowSize)
                        end if
                    end repeat
                end tell
            end tell
            return "0,0,400,800"
            "#,
            device_name
        );

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to get window bounds: {}", e)))?;

        if output.status.success() {
            let result = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let parts: Vec<&str> = result.split(',').collect();

            if parts.len() == 4 {
                let x = parts[0].parse::<f64>().unwrap_or(0.0);
                let y = parts[1].parse::<f64>().unwrap_or(0.0);
                let width = parts[2].parse::<f64>().unwrap_or(400.0);
                let height = parts[3].parse::<f64>().unwrap_or(800.0);

                eprintln!(
                    "[AppleScriptTap] Window bounds: x={}, y={}, w={}, h={}",
                    x, y, width, height
                );
                Ok((x, y, width, height))
            } else {
                eprintln!("[AppleScriptTap] Using default window bounds");
                Ok((0.0, 0.0, 400.0, 800.0))
            }
        } else {
            eprintln!("[AppleScriptTap] Failed to get window bounds, using defaults");
            Ok((0.0, 0.0, 400.0, 800.0))
        }
    }

    /// Get device name from device ID
    async fn get_device_name(device_id: &str) -> Result<String> {
        let output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "-j"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list devices: {}", e)))?;

        if let Ok(devices) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            for (_runtime, device_list) in devices["devices"]
                .as_object()
                .unwrap_or(&serde_json::Map::new())
            {
                if let Some(devices_array) = device_list.as_array() {
                    for device in devices_array {
                        if device["udid"].as_str() == Some(device_id) {
                            if let Some(name) = device["name"].as_str() {
                                return Ok(name.to_string());
                            }
                        }
                    }
                }
            }
        }

        Ok("iPhone".to_string()) // Default fallback
    }
}
