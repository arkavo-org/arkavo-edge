use crate::mcp::xcode_version::XcodeVersion;
use crate::{Result, TestError};
use serde_json::json;
use std::process::Command;

/// Handles simulator UI interactions with version-aware command selection
pub struct SimulatorInteraction {
    xcode_version: Option<XcodeVersion>,
}

impl SimulatorInteraction {
    pub fn new() -> Self {
        Self {
            xcode_version: XcodeVersion::detect().ok(),
        }
    }

    /// Perform a tap at the specified coordinates
    pub async fn tap(&self, device_id: &str, x: f64, y: f64) -> Result<serde_json::Value> {
        // Check Xcode version to determine available methods
        if let Some(version) = &self.xcode_version {
            if version.supports_enhanced_ui_interaction() {
                // Xcode 26+ has enhanced UI interaction
                return self.tap_enhanced(device_id, x, y).await;
            } else if version.supports_ui_commands() {
                // Xcode 15+ has basic UI commands
                return self.tap_ui_command(device_id, x, y).await;
            }
        }

        // Fallback to XCTest or other methods
        self.tap_fallback(device_id, x, y).await
    }

    /// Enhanced tap for Xcode 26+
    async fn tap_enhanced(&self, device_id: &str, x: f64, y: f64) -> Result<serde_json::Value> {
        // Xcode 26 might have new simctl interaction commands
        // For now, we'll use the standard approach with better error handling

        // Try using simctl ui if available
        let output = Command::new("xcrun")
            .args([
                "simctl",
                "ui",
                device_id,
                "tap",
                &x.to_string(),
                &y.to_string(),
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute enhanced tap: {}", e)))?;

        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "enhanced_ui_tap",
                "coordinates": {"x": x, "y": y},
                "device_id": device_id,
                "xcode_version": "26+"
            }))
        } else {
            // If the enhanced method fails, try the standard method
            self.tap_ui_command(device_id, x, y).await
        }
    }

    /// Standard UI tap for Xcode 15+
    async fn tap_ui_command(&self, device_id: &str, x: f64, y: f64) -> Result<serde_json::Value> {
        // For Xcode 15+, we might have ui commands
        // But simctl doesn't actually have a tap command, so we use alternative methods

        // Use AppleScript to send click events to the Simulator app
        let script = format!(
            r#"
            tell application "Simulator"
                activate
            end tell
            
            tell application "System Events"
                tell process "Simulator"
                    click at {{{}, {}}}
                end tell
            end tell
            "#,
            x, y
        );

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute AppleScript tap: {}", e)))?;

        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "applescript_tap",
                "coordinates": {"x": x, "y": y},
                "device_id": device_id
            }))
        } else {
            Err(TestError::Mcp(format!(
                "AppleScript tap failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )))
        }
    }

    /// Fallback tap method for older Xcode versions
    async fn tap_fallback(&self, _device_id: &str, _x: f64, _y: f64) -> Result<serde_json::Value> {
        // For older versions, we need to use XCTest or other automation frameworks
        Err(TestError::Mcp(
            "UI interaction requires Xcode 15 or later, or XCTest framework setup".to_string(),
        ))
    }

    /// Send text input to the simulator
    pub async fn send_text(&self, device_id: &str, text: &str) -> Result<serde_json::Value> {
        if let Some(version) = &self.xcode_version {
            if version.supports_enhanced_ui_interaction() {
                // Xcode 26+ might have better text input methods
                return self.send_text_enhanced(device_id, text).await;
            }
        }

        // Use standard pasteboard approach
        self.send_text_pasteboard(device_id, text).await
    }

    /// Enhanced text input for Xcode 26+
    async fn send_text_enhanced(&self, device_id: &str, text: &str) -> Result<serde_json::Value> {
        // Try using simctl ui sendkeys if available in Xcode 26
        let output = Command::new("xcrun")
            .args(["simctl", "ui", device_id, "sendkeys", text])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to send text: {}", e)))?;

        if output.status.success() {
            Ok(json!({
                "success": true,
                "method": "enhanced_sendkeys",
                "text": text,
                "device_id": device_id
            }))
        } else {
            // Fallback to pasteboard method
            self.send_text_pasteboard(device_id, text).await
        }
    }

    /// Standard text input using pasteboard
    async fn send_text_pasteboard(&self, device_id: &str, text: &str) -> Result<serde_json::Value> {
        // Set the pasteboard content
        let output = Command::new("xcrun")
            .args(["simctl", "pbcopy", device_id])
            .env("SIMCTL_CHILD_STDIN", text)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to set pasteboard: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to set pasteboard: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        // Send paste command using AppleScript
        let script = r#"
        tell application "Simulator"
            activate
        end tell
        
        tell application "System Events"
            keystroke "v" using command down
        end tell
        "#;

        let paste_output = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to execute paste: {}", e)))?;

        if paste_output.status.success() {
            Ok(json!({
                "success": true,
                "method": "pasteboard_paste",
                "text": text,
                "device_id": device_id
            }))
        } else {
            Err(TestError::Mcp(format!(
                "Failed to paste text: {}",
                String::from_utf8_lossy(&paste_output.stderr)
            )))
        }
    }

    /// Get version information
    pub fn get_version_info(&self) -> serde_json::Value {
        if let Some(version) = &self.xcode_version {
            json!({
                "xcode_version": format!("{}.{}.{}", version.major, version.minor, version.patch),
                "features": {
                    "bootstatus": version.supports_bootstatus(),
                    "privacy": version.supports_privacy(),
                    "ui_commands": version.supports_ui_commands(),
                    "device_appearance": version.supports_device_appearance(),
                    "push_notification": version.supports_push_notification(),
                    "clone": version.supports_clone(),
                    "device_pair": version.supports_device_pair(),
                    "device_focus": version.supports_device_focus(),
                    "device_streaming": version.supports_device_streaming(),
                    "enhanced_ui_interaction": version.supports_enhanced_ui_interaction(),
                }
            })
        } else {
            json!({
                "xcode_version": "unknown",
                "features": {}
            })
        }
    }
}

impl Default for SimulatorInteraction {
    fn default() -> Self {
        Self::new()
    }
}
