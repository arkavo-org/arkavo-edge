// idb_companion - iOS Debug Bridge Companion
// Copyright (c) Meta Platforms, Inc. and affiliates.
// Licensed under the MIT License
//
// This module embeds and wraps the idb_companion binary from Meta's idb project.
// See THIRD-PARTY-LICENSES.md for full license text.

use once_cell::sync::Lazy;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use crate::{Result, TestError};

// Embed the idb_companion binary at compile time
#[cfg(target_os = "macos")]
static IDB_COMPANION_BYTES: &[u8] = include_bytes!(env!("IDB_COMPANION_PATH"));

// Provide empty bytes for non-macOS platforms
#[cfg(not(target_os = "macos"))]
static IDB_COMPANION_BYTES: &[u8] = &[];

// Global path to extracted binary
static EXTRACTED_IDB_PATH: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

// Track connected devices for idb_companion
static CONNECTED_DEVICES: Lazy<Mutex<std::collections::HashSet<String>>> = 
    Lazy::new(|| Mutex::new(std::collections::HashSet::new()));

/// Wrapper around the embedded idb_companion binary
pub struct IdbWrapper;

impl IdbWrapper {
    /// Initialize idb_companion by extracting it to a temporary location
    pub fn initialize() -> Result<()> {
        #[cfg(not(target_os = "macos"))]
        {
            return Err(TestError::Mcp(
                "idb_companion is only supported on macOS".to_string(),
            ));
        }

        #[cfg(target_os = "macos")]
        {
            let mut path_guard = EXTRACTED_IDB_PATH.lock().unwrap();

            if path_guard.is_some() {
                // Already initialized
                return Ok(());
            }

            // Check if we have a real binary or just a placeholder
            if IDB_COMPANION_BYTES.len() < 1000 || IDB_COMPANION_BYTES.starts_with(b"#!/bin/bash") {
                return Err(TestError::Mcp(
                    "idb_companion not properly embedded. Install with: brew install idb-companion"
                        .to_string(),
                ));
            }

            // Create a temporary directory for the binary
            let temp_dir = std::env::temp_dir().join("arkavo_idb");
            fs::create_dir_all(&temp_dir)
                .map_err(|e| TestError::Mcp(format!("Failed to create temp dir: {}", e)))?;

            let binary_path = temp_dir.join("idb_companion");

            // Extract the binary
            fs::write(&binary_path, IDB_COMPANION_BYTES)
                .map_err(|e| TestError::Mcp(format!("Failed to extract idb_companion: {}", e)))?;

            // Make it executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&binary_path).unwrap().permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&binary_path, perms)
                    .map_err(|e| TestError::Mcp(format!("Failed to set permissions: {}", e)))?;
            }

            eprintln!(
                "[IdbWrapper] Extracted idb_companion to: {}",
                binary_path.display()
            );

            *path_guard = Some(binary_path);
            Ok(())
        }
    }

    /// Get the path to the extracted idb_companion binary
    fn get_binary_path() -> Result<PathBuf> {
        let path_guard = EXTRACTED_IDB_PATH.lock().unwrap();
        path_guard
            .as_ref()
            .cloned()
            .ok_or_else(|| TestError::Mcp("idb_companion not initialized".to_string()))
    }
    
    /// Connect to a device if not already connected (based on testing agent findings)
    fn ensure_connected(device_id: &str) -> Result<()> {
        let mut connected = CONNECTED_DEVICES.lock().unwrap();
        
        if connected.contains(device_id) {
            return Ok(());
        }
        
        eprintln!("[IdbWrapper] Connecting to device {}...", device_id);
        
        // First, check if we need to connect using idb list-targets
        // We'll use the system idb command for this check (as testing agent did)
        let list_output = Command::new("idb")
            .args(["list-targets"])
            .output();
            
        let needs_connection = if let Ok(output) = list_output {
            if output.status.success() {
                let output_str = String::from_utf8_lossy(&output.stdout);
                // Check if this device is already connected
                if output_str.contains(device_id) && output_str.contains("No Companion Connected") {
                    true
                } else if output_str.contains(device_id) {
                    eprintln!("[IdbWrapper] Device already connected via idb");
                    connected.insert(device_id.to_string());
                    false
                } else {
                    true
                }
            } else {
                true
            }
        } else {
            // If idb command fails, assume we need to connect
            true
        };
        
        if needs_connection {
            // Connect using system idb command (like testing agent)
            eprintln!("[IdbWrapper] Connecting via system idb command...");
            let connect_output = Command::new("idb")
                .args(["connect", device_id])
                .output();
                
            if let Ok(output) = connect_output {
                if output.status.success() {
                    eprintln!("[IdbWrapper] Successfully connected to device via idb");
                    connected.insert(device_id.to_string());
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    eprintln!("[IdbWrapper] Warning: idb connect failed: {}", stderr);
                    // Continue anyway - idb_companion might still work
                }
            } else {
                eprintln!("[IdbWrapper] Warning: Could not run idb connect command");
                // Continue anyway
            }
        }
        
        Ok(())
    }

    /// Perform a tap at the specified coordinates
    pub async fn tap(device_id: &str, x: f64, y: f64) -> Result<serde_json::Value> {
        eprintln!(
            "[IdbWrapper::tap] Tapping at ({}, {}) on device {}",
            x, y, device_id
        );

        // Method 1: Try idb_companion first (if properly embedded)
        if let Ok(()) = Self::initialize() {
            if let Ok(binary_path) = Self::get_binary_path() {
                Self::ensure_connected(device_id)?;
                
                let args = [
                    "ui",
                    "tap",
                    &x.to_string(),
                    &y.to_string(),
                    "--udid",
                    device_id,
                ];
                eprintln!("[IdbWrapper::tap] Trying idb_companion: {} {}", binary_path.display(), args.join(" "));
                
                let output = Command::new(&binary_path)
                    .args(&args)
                    .output();

                if let Ok(output) = output {
                    if output.status.success() {
                        eprintln!("[IdbWrapper::tap] idb_companion tap succeeded!");
                        return Ok(json!({
                            "success": true,
                            "method": "idb_companion",
                            "action": "tap",
                            "coordinates": {"x": x, "y": y},
                            "device_id": device_id,
                            "confidence": "high"
                        }));
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        eprintln!("[IdbWrapper::tap] idb_companion failed: {}", stderr);
                    }
                } else {
                    eprintln!("[IdbWrapper::tap] Failed to execute idb_companion");
                }
            }
        }

        // Method 2: Try system idb command (as testing agent used)
        eprintln!("[IdbWrapper::tap] Trying system idb command...");
        
        // First ensure we're connected (like testing agent did)
        let connect_output = Command::new("idb")
            .args(["connect", device_id])
            .output();
            
        if let Ok(output) = connect_output {
            if output.status.success() {
                eprintln!("[IdbWrapper::tap] Connected to device via system idb");
            } else {
                eprintln!("[IdbWrapper::tap] Connection attempt: {}", String::from_utf8_lossy(&output.stderr));
            }
        }
        
        // Now try the tap command
        let idb_output = Command::new("idb")
            .args([
                "ui",
                "tap",
                &x.to_string(),
                &y.to_string(),
                "--udid",
                device_id,
            ])
            .output();

        if let Ok(output) = idb_output {
            if output.status.success() {
                eprintln!("[IdbWrapper::tap] System idb tap succeeded!");
                return Ok(json!({
                    "success": true,
                    "method": "idb_system",
                    "action": "tap",
                    "coordinates": {"x": x, "y": y},
                    "device_id": device_id,
                    "confidence": "high"
                }));
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                eprintln!("[IdbWrapper::tap] System idb failed: {}", stderr);
            }
        } else {
            eprintln!("[IdbWrapper::tap] System idb command not available");
        }

        // Method 3: Try AppleScript (reliable fallback for simulator)
        eprintln!("[IdbWrapper::tap] Trying AppleScript method...");
        
        let applescript = format!(
            r#"
            tell application "Simulator"
                activate
                delay 0.5
            end tell
            
            tell application "System Events"
                tell process "Simulator"
                    click at {{{}, {}}}
                end tell
            end tell
            "#,
            x, y
        );
        
        let apple_output = Command::new("osascript")
            .args(["-e", &applescript])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute AppleScript: {}", e)))?;

        if apple_output.status.success() {
            eprintln!("[IdbWrapper::tap] AppleScript tap succeeded!");
            Ok(json!({
                "success": true,
                "method": "applescript",
                "action": "tap",
                "coordinates": {"x": x, "y": y},
                "device_id": device_id,
                "confidence": "medium"
            }))
        } else {
            let stderr = String::from_utf8_lossy(&apple_output.stderr);
            eprintln!("[IdbWrapper::tap] All tap methods failed, AppleScript error: {}", stderr);
            
            Err(TestError::Mcp(format!(
                "All tap methods failed. Last error from AppleScript: {}",
                stderr
            )))
        }
    }

    /// Perform a swipe gesture
    pub async fn swipe(
        device_id: &str,
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        duration: f64,
    ) -> Result<serde_json::Value> {
        Self::initialize()?;
        Self::ensure_connected(device_id)?;
        let binary_path = Self::get_binary_path()?;

        eprintln!(
            "[IdbWrapper] Swiping from ({}, {}) to ({}, {}) over {}s",
            start_x, start_y, end_x, end_y, duration
        );

        // Execute idb_companion swipe command
        let output = Command::new(&binary_path)
            .args([
                "ui",
                "swipe",
                &start_x.to_string(),
                &start_y.to_string(),
                &end_x.to_string(),
                &end_y.to_string(),
                "--duration",
                &duration.to_string(),
                "--udid",
                device_id,
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute idb_companion: {}", e)))?;

        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "idb_companion",
                "action": "swipe",
                "start": {"x": start_x, "y": start_y},
                "end": {"x": end_x, "y": end_y},
                "duration": duration,
                "device_id": device_id,
                "confidence": "high"
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("[IdbWrapper] Swipe failed: {}", stderr);

            Err(TestError::Mcp(format!(
                "idb_companion swipe failed: {}",
                stderr
            )))
        }
    }

    /// Type text into the currently focused element
    pub async fn type_text(device_id: &str, text: &str) -> Result<serde_json::Value> {
        Self::initialize()?;
        Self::ensure_connected(device_id)?;
        let binary_path = Self::get_binary_path()?;

        eprintln!(
            "[IdbWrapper] Typing text: '{}' on device {}",
            text, device_id
        );

        // Execute idb_companion text command
        let output = Command::new(&binary_path)
            .args(["ui", "text", text, "--udid", device_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute idb_companion: {}", e)))?;

        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "idb_companion",
                "action": "type_text",
                "text": text,
                "device_id": device_id,
                "confidence": "high"
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("[IdbWrapper] Type text failed: {}", stderr);

            Err(TestError::Mcp(format!(
                "idb_companion type_text failed: {}",
                stderr
            )))
        }
    }

    /// Press a hardware button (e.g., "home", "power", "volumeup", "volumedown")
    pub async fn press_button(device_id: &str, button: &str) -> Result<serde_json::Value> {
        Self::initialize()?;
        Self::ensure_connected(device_id)?;
        let binary_path = Self::get_binary_path()?;

        eprintln!(
            "[IdbWrapper] Pressing button: '{}' on device {}",
            button, device_id
        );

        // Execute idb_companion button command
        let output = Command::new(&binary_path)
            .args(["ui", "button", button, "--udid", device_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute idb_companion: {}", e)))?;

        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "idb_companion",
                "action": "press_button",
                "button": button,
                "device_id": device_id,
                "confidence": "high"
            }))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            eprintln!("[IdbWrapper] Press button failed: {}", stderr);

            Err(TestError::Mcp(format!(
                "idb_companion press_button failed: {}",
                stderr
            )))
        }
    }

    /// Clean up extracted binary on drop
    pub fn cleanup() {
        if let Ok(mut path_guard) = EXTRACTED_IDB_PATH.lock() {
            if let Some(path) = path_guard.take() {
                let _ = fs::remove_file(&path);
                eprintln!("[IdbWrapper] Cleaned up extracted binary");
            }
        }
        
        if let Ok(mut connected) = CONNECTED_DEVICES.lock() {
            eprintln!("[IdbWrapper] Disconnecting from {} devices", connected.len());
            connected.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_idb_wrapper_initialization() {
        // This test will fail on non-macOS platforms as expected
        let result = IdbWrapper::initialize();

        #[cfg(target_os = "macos")]
        {
            // On macOS, initialization should succeed (though the placeholder will fail)
            match result {
                Ok(_) => eprintln!("idb_companion initialized successfully"),
                Err(e) => eprintln!("idb_companion not available: {}", e),
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            // On other platforms, should return error
            assert!(result.is_err());
        }
    }
}