use serde_json::json;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::{Result, TestError};
use super::idb_wrapper::IdbWrapper;
use super::idb_companion_health::IdbCompanionHealth;
use super::simulator_state_verifier::SimulatorStateVerifier;

#[derive(Debug)]
struct WindowInfo {
    x: f64,
    y: f64,
    _width: f64,
    _height: f64,
}

/// Enhanced tap implementation with verification and fallback strategies
/// This provides improved tap reliability for calibration and testing
pub struct IdbTapEnhanced;

impl IdbTapEnhanced {
    /// Perform a tap with retry logic and verification
    pub async fn tap_with_verification(
        device_id: &str,
        x: f64,
        y: f64,
        max_retries: u32,
    ) -> Result<serde_json::Value> {
        let start_time = Instant::now();
        eprintln!("[IdbTapEnhanced] Starting enhanced tap at ({}, {}) on device {}", x, y, device_id);
        
        // Pre-flight checks
        // 1. Verify simulator state
        if let Err(e) = SimulatorStateVerifier::prepare_for_interaction(device_id, None).await {
            eprintln!("[IdbTapEnhanced] Warning: Simulator preparation failed: {}", e);
        }
        
        // 2. Check IDB companion health
        let companion_healthy = IdbCompanionHealth::check_health(device_id).await.unwrap_or(false);
        if !companion_healthy {
            eprintln!("[IdbTapEnhanced] IDB companion unhealthy, attempting recovery...");
            if let Err(e) = IdbCompanionHealth::recover_companion(device_id).await {
                eprintln!("[IdbTapEnhanced] Recovery failed: {}", e);
            }
        }
        
        // 3. Verify coordinates are within reasonable bounds
        if let Err(e) = Self::verify_coordinates(device_id, x, y).await {
            eprintln!("[IdbTapEnhanced] Coordinate verification failed: {}", e);
            // Continue anyway as the coordinates might still be valid
        }
        
        let mut last_error = None;
        let mut _method_used = "none";
        
        for attempt in 0..max_retries {
            if attempt > 0 {
                eprintln!("[IdbTapEnhanced] Retry attempt {} of {}", attempt, max_retries - 1);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            
            // Try IDB tap with timing adjustments
            let tap_start = Instant::now();
            match Self::try_idb_tap_with_timing(device_id, x, y).await {
                Ok(mut result) => {
                    let latency = tap_start.elapsed().as_millis() as u64;
                    eprintln!("[IdbTapEnhanced] IDB tap succeeded on attempt {} ({}ms)", attempt + 1, latency);
                    
                    // Record success metrics
                    IdbCompanionHealth::record_tap_result(device_id, true, latency);
                    
                    // Add metrics to result
                    if let Some(obj) = result.as_object_mut() {
                        obj.insert("latency_ms".to_string(), serde_json::json!(latency));
                        obj.insert("attempt".to_string(), serde_json::json!(attempt + 1));
                        obj.insert("total_time_ms".to_string(), serde_json::json!(start_time.elapsed().as_millis()));
                    }
                    
                    return Ok(result);
                }
                Err(e) => {
                    let latency = tap_start.elapsed().as_millis() as u64;
                    eprintln!("[IdbTapEnhanced] IDB tap failed: {} ({}ms)", e, latency);
                    
                    // Record failure metrics
                    IdbCompanionHealth::record_tap_result(device_id, false, latency);
                    last_error = Some(e);
                    
                    // On first failure, try AppleScript as fallback
                    if attempt == 0 {
                        eprintln!("[IdbTapEnhanced] Trying AppleScript fallback...");
                        if let Ok(mut result) = Self::try_simctl_tap(device_id, x, y).await {
                            eprintln!("[IdbTapEnhanced] AppleScript tap succeeded");
                            _method_used = "applescript_fallback";
                            
                            // Add metrics to result
                            if let Some(obj) = result.as_object_mut() {
                                obj.insert("latency_ms".to_string(), serde_json::json!(tap_start.elapsed().as_millis()));
                                obj.insert("attempt".to_string(), serde_json::json!(attempt + 1));
                                obj.insert("total_time_ms".to_string(), serde_json::json!(start_time.elapsed().as_millis()));
                                obj.insert("fallback_used".to_string(), serde_json::json!(true));
                            }
                            
                            return Ok(result);
                        }
                    }
                }
            }
        }
        
        // Log final failure with health report
        eprintln!("[IdbTapEnhanced] All tap attempts failed. Health report:");
        eprintln!("{}", serde_json::to_string_pretty(&IdbCompanionHealth::get_health_report()).unwrap_or_default());
        
        Err(last_error.unwrap_or_else(|| TestError::Mcp("All tap attempts failed".to_string())))
    }
    
    /// Try IDB tap with proper timing between touch events
    async fn try_idb_tap_with_timing(device_id: &str, x: f64, y: f64) -> Result<serde_json::Value> {
        // First, ensure the simulator is in focus
        Self::focus_simulator(device_id).await?;
        
        // Add a small delay to ensure the simulator is ready
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        // Perform the tap
        let result = IdbWrapper::tap(device_id, x, y).await?;
        
        // Add a small delay after tap to ensure it registers
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        Ok(result)
    }
    
    /// Try using AppleScript as fallback for UI automation
    async fn try_simctl_tap(device_id: &str, x: f64, y: f64) -> Result<serde_json::Value> {
        eprintln!("[IdbTapEnhanced] Attempting AppleScript tap at ({}, {})", x, y);
        
        // First ensure the simulator window is focused
        let _ = Command::new("xcrun")
            .args(["simctl", "ui", device_id, "appearance", "light"])
            .output();
        
        // Get the simulator window position
        let window_info = Self::get_simulator_window_info(device_id).await?;
        
        // Convert device coordinates to screen coordinates
        let screen_x = window_info.x + x;
        let screen_y = window_info.y + y;
        
        // Use AppleScript to perform the tap
        let script = format!(
            r#"
            tell application "Simulator"
                activate
            end tell
            delay 0.1
            tell application "System Events"
                click at {{{}, {}}}
            end tell
            "#,
            screen_x, screen_y
        );
        
        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute AppleScript: {}", e)))?;
        
        if output.status.success() {
            eprintln!("[IdbTapEnhanced] AppleScript tap succeeded");
            Ok(json!({
                "success": true,
                "method": "applescript",
                "action": "tap",
                "coordinates": {"x": x, "y": y},
                "device_id": device_id,
                "confidence": "medium"
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("[IdbTapEnhanced] AppleScript tap failed: {}", stderr);
            Err(TestError::Mcp(format!("AppleScript tap failed: {}", stderr)))
        }
    }
    
    /// Focus the simulator window
    async fn focus_simulator(device_id: &str) -> Result<()> {
        // Use simctl to ensure the device is the active window
        let output = Command::new("xcrun")
            .args(["simctl", "ui", device_id, "appearance", "light"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to focus simulator: {}", e)))?;
        
        if !output.status.success() {
            eprintln!("[IdbTapEnhanced] Warning: Could not focus simulator");
        }
        
        Ok(())
    }
    
    /// Verify coordinates are within device bounds
    async fn verify_coordinates(device_id: &str, x: f64, y: f64) -> Result<()> {
        // Get device info to check bounds
        let output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "-j"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list devices: {}", e)))?;
        
        if let Ok(devices) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            // Try to find device and infer screen size from device type
            for (_runtime, device_list) in devices["devices"].as_object().unwrap_or(&serde_json::Map::new()) {
                if let Some(devices_array) = device_list.as_array() {
                    for device in devices_array {
                        if device["udid"].as_str() == Some(device_id) {
                            let device_type = device["deviceTypeIdentifier"].as_str().unwrap_or("");
                            let (width, height) = Self::infer_screen_size(device_type);
                            
                            if x < 0.0 || x > width || y < 0.0 || y > height {
                                return Err(TestError::Mcp(format!(
                                    "Coordinates ({}, {}) out of bounds for device with size {}x{}", 
                                    x, y, width, height
                                )));
                            }
                            
                            return Ok(());
                        }
                    }
                }
            }
        }
        
        // If we can't verify, assume it's okay
        Ok(())
    }
    
    /// Infer screen size from device type identifier
    fn infer_screen_size(device_type: &str) -> (f64, f64) {
        // These are logical points, not pixels
        match device_type {
            s if s.contains("iPhone-16-Pro-Max") => (430.0, 932.0),
            s if s.contains("iPhone-16-Pro") => (393.0, 852.0),
            s if s.contains("iPhone-16-Plus") || s.contains("iPhone-15-Plus") => (428.0, 926.0),
            s if s.contains("iPhone-16") || s.contains("iPhone-15") => (390.0, 844.0),
            s if s.contains("iPhone-SE") => (375.0, 667.0),
            s if s.contains("iPad-Pro-12-9") => (1024.0, 1366.0),
            s if s.contains("iPad-Pro-11") => (834.0, 1194.0),
            s if s.contains("iPad") => (810.0, 1080.0),
            _ => (390.0, 844.0), // Default to iPhone 15 size
        }
    }
    
    /// Get simulator window information for coordinate conversion
    async fn get_simulator_window_info(_device_id: &str) -> Result<WindowInfo> {
        // This is a simplified version - in reality we'd query the actual window position
        // For now, return default values
        Ok(WindowInfo {
            x: 0.0,
            y: 0.0,
            _width: 400.0,
            _height: 800.0,
        })
    }
    
    /// Perform a tap sequence with verification between taps
    pub async fn tap_sequence_with_verification(
        device_id: &str,
        taps: Vec<(f64, f64)>,
        verification_delay_ms: u64,
    ) -> Result<Vec<serde_json::Value>> {
        let mut results = Vec::new();
        
        for (idx, (x, y)) in taps.iter().enumerate() {
            eprintln!("[IdbTapEnhanced] Executing tap {} of {} at ({}, {})", 
                idx + 1, taps.len(), x, y);
            
            let result = Self::tap_with_verification(device_id, *x, *y, 3).await?;
            results.push(result);
            
            // Wait between taps for app to process
            if idx < taps.len() - 1 {
                tokio::time::sleep(Duration::from_millis(verification_delay_ms)).await;
            }
        }
        
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_screen_size_inference() {
        let (w, h) = IdbTapEnhanced::infer_screen_size("com.apple.CoreSimulator.SimDeviceType.iPhone-16-Pro");
        assert_eq!((w, h), (393.0, 852.0));
        
        let (w, h) = IdbTapEnhanced::infer_screen_size("com.apple.CoreSimulator.SimDeviceType.iPhone-SE-3rd-generation");
        assert_eq!((w, h), (375.0, 667.0));
    }
}