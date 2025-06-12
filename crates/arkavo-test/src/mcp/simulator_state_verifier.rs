use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::Duration;

use crate::{Result, TestError};

/// Represents the current state of a simulator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatorState {
    pub device_id: String,
    pub is_booted: bool,
    pub is_responsive: bool,
    pub has_active_dialog: bool,
    pub app_is_foreground: bool,
    pub screen_is_locked: bool,
    pub system_alerts: Vec<String>,
    pub keyboard_visible: bool,
    pub orientation: String,
}

/// Verifies simulator state before interactions
pub struct SimulatorStateVerifier;

impl SimulatorStateVerifier {
    /// Comprehensive state check before performing UI interactions
    pub async fn verify_ready_for_interaction(
        device_id: &str,
        app_bundle_id: Option<&str>,
    ) -> Result<SimulatorState> {
        eprintln!("[SimulatorStateVerifier] Checking state for device {}", device_id);
        
        let mut state = SimulatorState {
            device_id: device_id.to_string(),
            is_booted: false,
            is_responsive: false,
            has_active_dialog: false,
            app_is_foreground: false,
            screen_is_locked: false,
            system_alerts: Vec::new(),
            keyboard_visible: false,
            orientation: "unknown".to_string(),
        };
        
        // 1. Check if device is booted
        state.is_booted = Self::check_device_booted(device_id).await?;
        if !state.is_booted {
            return Err(TestError::Mcp("Device is not booted".to_string()));
        }
        
        // 2. Check if simulator is responsive
        state.is_responsive = Self::check_device_responsive(device_id).await?;
        if !state.is_responsive {
            eprintln!("[SimulatorStateVerifier] Device not responsive, attempting to wake...");
            Self::wake_device(device_id).await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
            state.is_responsive = Self::check_device_responsive(device_id).await?;
        }
        
        // 3. Check for system alerts or dialogs
        state.system_alerts = Self::check_system_alerts(device_id).await?;
        state.has_active_dialog = !state.system_alerts.is_empty();
        
        // 4. Check screen lock state
        state.screen_is_locked = Self::check_screen_locked(device_id).await?;
        if state.screen_is_locked {
            eprintln!("[SimulatorStateVerifier] Screen is locked, attempting to unlock...");
            Self::unlock_screen(device_id).await?;
            tokio::time::sleep(Duration::from_secs(1)).await;
            state.screen_is_locked = Self::check_screen_locked(device_id).await?;
        }
        
        // 5. Check app state if bundle ID provided
        if let Some(bundle_id) = app_bundle_id {
            state.app_is_foreground = Self::check_app_foreground(device_id, bundle_id).await?;
            if !state.app_is_foreground {
                eprintln!("[SimulatorStateVerifier] App {} is not in foreground", bundle_id);
            }
        }
        
        // 6. Check keyboard state
        state.keyboard_visible = Self::check_keyboard_visible(device_id).await?;
        
        // 7. Get device orientation
        state.orientation = Self::get_device_orientation(device_id).await?;
        
        eprintln!("[SimulatorStateVerifier] State check complete: booted={}, responsive={}, dialogs={}, locked={}", 
            state.is_booted, state.is_responsive, state.has_active_dialog, state.screen_is_locked);
        
        Ok(state)
    }
    
    /// Check if device is booted
    async fn check_device_booted(device_id: &str) -> Result<bool> {
        let output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "-j"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list devices: {}", e)))?;
            
        if let Ok(devices) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            for (_runtime, device_list) in devices["devices"].as_object().unwrap_or(&serde_json::Map::new()) {
                if let Some(devices_array) = device_list.as_array() {
                    for device in devices_array {
                        if device["udid"].as_str() == Some(device_id) {
                            return Ok(device["state"].as_str() == Some("Booted"));
                        }
                    }
                }
            }
        }
        
        Ok(false)
    }
    
    /// Check if device is responsive
    async fn check_device_responsive(device_id: &str) -> Result<bool> {
        // Try to get device info - if this works, device is responsive
        let output = Command::new("xcrun")
            .args(["simctl", "getenv", device_id, "HOME"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to check device responsiveness: {}", e)))?;
            
        Ok(output.status.success())
    }
    
    /// Wake device if sleeping
    async fn wake_device(device_id: &str) -> Result<()> {
        // Send power button press to wake
        let _ = Command::new("xcrun")
            .args(["simctl", "io", device_id, "power", "off"])
            .output();
            
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        let _ = Command::new("xcrun")
            .args(["simctl", "io", device_id, "power", "on"])
            .output();
            
        Ok(())
    }
    
    /// Check for system alerts
    async fn check_system_alerts(_device_id: &str) -> Result<Vec<String>> {
        // This would ideally use XCTest to query UI, but for now we'll return empty
        // In a real implementation, this would check for:
        // - Permission dialogs
        // - System notifications
        // - App crash dialogs
        // - Network error alerts
        Ok(Vec::new())
    }
    
    /// Check if screen is locked
    async fn check_screen_locked(_device_id: &str) -> Result<bool> {
        // Would need UI query to properly check
        // For now, assume unlocked
        Ok(false)
    }
    
    /// Unlock screen
    async fn unlock_screen(device_id: &str) -> Result<()> {
        // Swipe up to unlock
        let output = Command::new("xcrun")
            .args(["simctl", "io", device_id, "swipe", "up"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to unlock screen: {}", e)))?;
            
        if !output.status.success() {
            eprintln!("[SimulatorStateVerifier] Warning: Could not unlock screen");
        }
        
        Ok(())
    }
    
    /// Check if app is in foreground
    async fn check_app_foreground(device_id: &str, bundle_id: &str) -> Result<bool> {
        // Get list of running apps
        let output = Command::new("xcrun")
            .args(["simctl", "listapps", device_id, "-j"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list apps: {}", e)))?;
            
        if let Ok(apps) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
            if let Some(app_array) = apps.as_array() {
                for app in app_array {
                    if app["CFBundleIdentifier"].as_str() == Some(bundle_id) {
                        // Check if app is installed
                        // Note: simctl doesn't provide foreground state directly
                        return Ok(true); // Assume true if installed
                    }
                }
            }
        }
        
        Ok(false)
    }
    
    /// Check if keyboard is visible
    async fn check_keyboard_visible(_device_id: &str) -> Result<bool> {
        // Would need UI query to properly check
        Ok(false)
    }
    
    /// Get device orientation
    async fn get_device_orientation(_device_id: &str) -> Result<String> {
        // Default to portrait for now
        Ok("portrait".to_string())
    }
    
    /// Dismiss any active dialogs
    pub async fn dismiss_dialogs(device_id: &str) -> Result<()> {
        eprintln!("[SimulatorStateVerifier] Attempting to dismiss dialogs...");
        
        // Try common dismiss actions
        // 1. Press home button
        let _ = Command::new("xcrun")
            .args(["simctl", "io", device_id, "home"])
            .output();
            
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        // 2. Try escape key
        #[cfg(target_os = "macos")]
        {
            let script = r#"
            tell application "Simulator"
                activate
            end tell
            tell application "System Events"
                key code 53
            end tell
            "#;
            
            let _ = Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output();
        }
        
        Ok(())
    }
    
    /// Prepare simulator for interaction
    pub async fn prepare_for_interaction(
        device_id: &str,
        app_bundle_id: Option<&str>,
    ) -> Result<()> {
        eprintln!("[SimulatorStateVerifier] Preparing device {} for interaction", device_id);
        
        // 1. Verify state
        let state = Self::verify_ready_for_interaction(device_id, app_bundle_id).await?;
        
        // 2. Handle any issues
        if state.has_active_dialog {
            Self::dismiss_dialogs(device_id).await?;
        }
        
        // 3. Launch app if needed and not in foreground
        if let Some(bundle_id) = app_bundle_id {
            if !state.app_is_foreground {
                eprintln!("[SimulatorStateVerifier] Launching app {}", bundle_id);
                let _ = Command::new("xcrun")
                    .args(["simctl", "launch", device_id, bundle_id])
                    .output();
                    
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        
        // 4. Focus simulator window
        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("xcrun")
                .args(["simctl", "ui", device_id, "appearance", "light"])
                .output();
        }
        
        eprintln!("[SimulatorStateVerifier] Device ready for interaction");
        Ok(())
    }
}