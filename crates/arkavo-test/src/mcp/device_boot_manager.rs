use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::time::sleep;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootStatus {
    pub device_id: String,
    pub boot_duration_seconds: Option<f64>,
    pub current_state: BootState,
    pub services_ready: bool,
    pub ui_ready: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BootState {
    NotStarted,
    Booting,
    WaitingForServices,
    CheckingUI,
    Ready,
    Failed,
}

pub struct DeviceBootManager;

impl DeviceBootManager {
    /// Boot a device and wait for it to be fully ready
    pub async fn boot_device_with_wait(device_id: &str, timeout: Duration) -> Result<BootStatus> {
        let started_at = Instant::now();
        let mut status = BootStatus {
            device_id: device_id.to_string(),
            boot_duration_seconds: None,
            current_state: BootState::NotStarted,
            services_ready: false,
            ui_ready: false,
            error: None,
        };

        // Start the boot process
        status.current_state = BootState::Booting;
        let boot_output = Command::new("xcrun")
            .args(["simctl", "boot", device_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to start boot: {}", e)))?;

        // Check if already booted (this is not an error)
        if !boot_output.status.success() {
            let stderr = String::from_utf8_lossy(&boot_output.stderr);
            if stderr.contains("Unable to boot device in current state: Booted") {
                // Already booted, just verify it's ready
                status.current_state = BootState::CheckingUI;
            } else {
                status.current_state = BootState::Failed;
                status.error = Some(stderr.to_string());
                return Ok(status);
            }
        }

        // Wait for device to be ready
        let deadline = Instant::now() + timeout;

        // Phase 1: Wait for basic boot
        status.current_state = BootState::WaitingForServices;
        while std::time::Instant::now() < deadline {
            if Self::check_device_booted(device_id).await? {
                status.services_ready = true;
                break;
            }
            sleep(Duration::from_millis(500)).await;
        }

        if !status.services_ready {
            status.current_state = BootState::Failed;
            status.error = Some("Timeout waiting for device to boot".to_string());
            return Ok(status);
        }

        // Phase 2: Wait for UI services
        status.current_state = BootState::CheckingUI;
        while std::time::Instant::now() < deadline {
            if Self::check_ui_ready(device_id).await? {
                status.ui_ready = true;
                break;
            }
            sleep(Duration::from_millis(500)).await;
        }

        if !status.ui_ready {
            // UI might still be loading, but basic services are ready
            eprintln!(
                "Warning: UI services may not be fully ready on device {}",
                device_id
            );
        }

        status.current_state = BootState::Ready;
        status.boot_duration_seconds = Some(started_at.elapsed().as_secs_f64());

        Ok(status)
    }

    /// Check if device shows as booted in simctl
    async fn check_device_booted(device_id: &str) -> Result<bool> {
        let output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "-j"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list devices: {}", e)))?;

        let json_str = String::from_utf8_lossy(&output.stdout);
        let data: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| TestError::Mcp(format!("Failed to parse device list: {}", e)))?;

        // Find the device in the JSON
        if let Some(devices_map) = data.get("devices").and_then(|d| d.as_object()) {
            for (_runtime, device_list) in devices_map {
                if let Some(devices) = device_list.as_array() {
                    for device in devices {
                        if device.get("udid").and_then(|u| u.as_str()) == Some(device_id) {
                            return Ok(
                                device.get("state").and_then(|s| s.as_str()) == Some("Booted")
                            );
                        }
                    }
                }
            }
        }

        Ok(false)
    }

    /// Check if UI services are ready (SpringBoard is responsive)
    async fn check_ui_ready(device_id: &str) -> Result<bool> {
        // Try to query the UI state
        let output = Command::new("xcrun")
            .args([
                "simctl",
                "spawn",
                device_id,
                "launchctl",
                "print",
                "system/com.apple.SpringBoard",
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to check SpringBoard: {}", e)))?;

        // If SpringBoard service is running, UI should be ready
        Ok(output.status.success() && !output.stdout.is_empty())
    }

    /// Get current boot progress for a device
    pub async fn get_boot_progress(device_id: &str) -> Result<String> {
        let output = Command::new("xcrun")
            .args([
                "simctl", "spawn", device_id, "log", "show", "--style", "compact", "--last", "1m",
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to get device logs: {}", e)))?;

        let logs = String::from_utf8_lossy(&output.stdout);

        // Look for key boot milestones in logs
        let mut progress = Vec::new();

        if logs.contains("Boot begun") {
            progress.push("Boot initiated");
        }
        if logs.contains("SpringBoard") && logs.contains("application launched") {
            progress.push("SpringBoard launched");
        }
        if logs.contains("System app bundle scan completed") {
            progress.push("Apps scanned");
        }
        if logs.contains("Boot complete") {
            progress.push("Boot complete");
        }

        if progress.is_empty() {
            Ok("Boot progress unknown".to_string())
        } else {
            Ok(progress.join(" → "))
        }
    }

    /// Force terminate all simulator processes for a device
    pub async fn force_terminate_simulator(device_id: &str) -> Result<()> {
        Command::new("xcrun")
            .args(["simctl", "terminate", device_id, "all"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to terminate apps: {}", e)))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boot_status_serialization() {
        let status = BootStatus {
            device_id: "test-device".to_string(),
            boot_duration_seconds: Some(30.0),
            current_state: BootState::Ready,
            services_ready: true,
            ui_ready: true,
            error: None,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"current_state\":\"Ready\""));
        assert!(json.contains("\"services_ready\":true"));
    }
}
