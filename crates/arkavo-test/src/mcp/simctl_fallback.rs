use crate::{Result, TestError};
use serde_json::json;
use std::process::Command;

/// Fallback implementation for when IDB fails
/// Note: xcrun simctl doesn't have direct tap capability, so we use AppleScript
pub struct SimctlFallback;

impl SimctlFallback {
    /// Perform a tap using AppleScript to control the Simulator
    pub async fn tap(device_id: &str, x: f64, y: f64) -> Result<serde_json::Value> {
        eprintln!("[SimctlFallback] simctl doesn't support tap - using AppleScript fallback");
        
        // First ensure the device is booted
        Self::ensure_booted(device_id).await?;
        
        // Try to use AppleScript tap if available
        #[cfg(target_os = "macos")]
        {
            if let Ok(result) = crate::mcp::applescript_tap::AppleScriptTap::tap(device_id, x, y).await {
                return Ok(result);
            }
        }
        
        // If AppleScript fails or not on macOS, return error
        Err(TestError::Mcp(
            "simctl does not support tap operations. IDB or XCTest is required for UI automation.".to_string()
        ))
    }
    
    /// Take a screenshot using simctl
    pub async fn screenshot(device_id: &str, output_path: &str) -> Result<serde_json::Value> {
        eprintln!("[SimctlFallback] Taking screenshot using xcrun simctl");
        
        let output = Command::new("xcrun")
            .args([
                "simctl", 
                "io", 
                device_id, 
                "screenshot", 
                output_path
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute simctl screenshot: {}", e)))?;
            
        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "simctl_fallback",
                "action": "screenshot",
                "path": output_path,
                "device_id": device_id
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(TestError::Mcp(format!("simctl screenshot failed: {}", stderr)))
        }
    }
    
    /// Boot a device if needed
    pub async fn ensure_booted(device_id: &str) -> Result<()> {
        eprintln!("[SimctlFallback] Ensuring device {} is booted", device_id);
        
        // Check current state
        let list_output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "-j"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to run simctl list: {}", e)))?;
            
        if !list_output.status.success() {
            return Err(TestError::Mcp("Failed to list devices".to_string()));
        }
        
        let json = serde_json::from_slice::<serde_json::Value>(&list_output.stdout)
            .map_err(|e| TestError::Mcp(format!("Failed to parse device list: {}", e)))?;
            
        let mut needs_boot = false;
        
        for (_runtime, devices) in json["devices"].as_object().unwrap_or(&serde_json::Map::new()) {
            if let Some(device_array) = devices.as_array() {
                for device in device_array {
                    if device["udid"].as_str() == Some(device_id) {
                        let state = device["state"].as_str().unwrap_or("Unknown");
                        if state != "Booted" {
                            needs_boot = true;
                        }
                        break;
                    }
                }
            }
        }
        
        if needs_boot {
            eprintln!("[SimctlFallback] Booting device {}...", device_id);
            let boot_output = Command::new("xcrun")
                .args(["simctl", "boot", device_id])
                .output()
                .map_err(|e| TestError::Mcp(format!("Failed to boot device: {}", e)))?;
                
            if !boot_output.status.success() {
                let stderr = String::from_utf8_lossy(&boot_output.stderr);
                if !stderr.contains("Unable to boot device in current state: Booted") {
                    return Err(TestError::Mcp(format!("Failed to boot device: {}", stderr)));
                }
            }
            
            // Wait for boot
            tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
        }
        
        Ok(())
    }
}