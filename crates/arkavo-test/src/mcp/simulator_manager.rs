use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatorDevice {
    pub udid: String,
    pub name: String,
    pub state: String,
    pub device_type: String,
    pub runtime: String,
    pub is_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub bundle_id: String,
    pub name: String,
    pub path: String,
    pub version: String,
}

pub struct SimulatorManager {
    pub devices: HashMap<String, SimulatorDevice>,
}

impl SimulatorManager {
    pub fn new() -> Self {
        let mut manager = Self {
            devices: HashMap::new(),
        };
        manager.refresh_devices().ok();
        manager
    }

    pub fn refresh_devices(&mut self) -> Result<()> {
        let output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "--json"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list devices: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to list devices: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let data: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| TestError::Mcp(format!("Failed to parse device list: {}", e)))?;

        self.devices.clear();

        if let Some(devices_by_runtime) = data.get("devices").and_then(|d| d.as_object()) {
            for (runtime, devices) in devices_by_runtime {
                if let Some(device_array) = devices.as_array() {
                    for device in device_array {
                        if let Some(udid) = device.get("udid").and_then(|u| u.as_str()) {
                            let sim_device = SimulatorDevice {
                                udid: udid.to_string(),
                                name: device
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("Unknown")
                                    .to_string(),
                                state: device
                                    .get("state")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("Unknown")
                                    .to_string(),
                                device_type: device
                                    .get("deviceTypeIdentifier")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("Unknown")
                                    .to_string(),
                                runtime: runtime.clone(),
                                is_available: device
                                    .get("isAvailable")
                                    .and_then(|a| a.as_bool())
                                    .unwrap_or(false),
                            };
                            self.devices.insert(udid.to_string(), sim_device);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn get_device(&self, udid: &str) -> Option<&SimulatorDevice> {
        self.devices.get(udid)
    }

    pub fn get_booted_devices(&self) -> Vec<&SimulatorDevice> {
        self.devices
            .values()
            .filter(|d| d.state == "Booted")
            .collect()
    }

    pub fn boot_device(&self, udid: &str) -> Result<()> {
        let output = Command::new("xcrun")
            .args(["simctl", "boot", udid])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to boot device: {}", e)))?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            if error.contains("already booted") {
                return Ok(());
            }
            return Err(TestError::Mcp(format!("Failed to boot device: {}", error)));
        }

        Ok(())
    }

    pub fn shutdown_device(&self, udid: &str) -> Result<()> {
        let output = Command::new("xcrun")
            .args(["simctl", "shutdown", udid])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to shutdown device: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to shutdown device: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    pub fn install_app(&self, udid: &str, app_path: &str) -> Result<()> {
        // Check if device exists and is booted
        if let Some(device) = self.devices.get(udid) {
            if device.state != "Booted" {
                return Err(TestError::Mcp(format!(
                    "Device {} is not booted (current state: {}). Boot the device first.",
                    udid, device.state
                )));
            }
        } else {
            return Err(TestError::Mcp(format!(
                "Device {} not found. Use 'simulator_control' tool with 'list' action to see available devices.",
                udid
            )));
        }
        // Resolve wildcards in the path if present
        let resolved_path = if app_path.contains('*') {
            // Use glob to resolve the wildcard
            match ::glob::glob(app_path) {
                Ok(paths) => {
                    // Collect all matching paths
                    let mut matches: Vec<std::path::PathBuf> = paths
                        .filter_map(|r| r.ok())
                        .collect();
                    
                    if matches.is_empty() {
                        return Err(TestError::Mcp(format!(
                            "No app found matching pattern: {}",
                            app_path
                        )));
                    }
                    
                    // If multiple matches, use the most recently modified
                    if matches.len() > 1 {
                        matches.sort_by_key(|path| {
                            std::fs::metadata(path)
                                .and_then(|m| m.modified())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                        matches.reverse(); // Most recent first
                        
                        eprintln!(
                            "Multiple apps found matching '{}', using most recent: {}",
                            app_path,
                            matches[0].display()
                        );
                    }
                    
                    matches[0].to_string_lossy().to_string()
                }
                Err(e) => {
                    return Err(TestError::Mcp(format!(
                        "Invalid glob pattern '{}': {}",
                        app_path, e
                    )));
                }
            }
        } else {
            app_path.to_string()
        };

        // Verify the resolved path exists
        if !std::path::Path::new(&resolved_path).exists() {
            return Err(TestError::Mcp(format!(
                "App not found at resolved path: {}",
                resolved_path
            )));
        }

        let output = Command::new("xcrun")
            .args(["simctl", "install", udid, &resolved_path])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to install app: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to install app at '{}': {}",
                resolved_path,
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    pub fn uninstall_app(&self, udid: &str, bundle_id: &str) -> Result<()> {
        let output = Command::new("xcrun")
            .args(["simctl", "uninstall", udid, bundle_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to uninstall app: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to uninstall app: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    pub fn launch_app(&self, udid: &str, bundle_id: &str, args: &[&str]) -> Result<i32> {
        // Don't use --console flag as it can cause the command to hang
        let mut cmd_args = vec!["simctl", "launch", udid, bundle_id];
        cmd_args.extend_from_slice(args);

        let output = Command::new("xcrun")
            .args(&cmd_args)
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to launch app: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to launch app: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        // Parse PID from output like: "com.arkavo.Arkavo: 12345"
        let output_str = String::from_utf8_lossy(&output.stdout);
        if let Some(pid_str) = output_str.trim().split(": ").nth(1) {
            if let Ok(pid) = pid_str.parse::<i32>() {
                return Ok(pid);
            }
        }

        // Return 0 if we can't parse PID but launch was successful
        Ok(0)
    }

    pub fn terminate_app(&self, udid: &str, bundle_id: &str) -> Result<()> {
        let output = Command::new("xcrun")
            .args(["simctl", "terminate", udid, bundle_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to terminate app: {}", e)))?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            if error.contains("not running") {
                return Ok(());
            }
            return Err(TestError::Mcp(format!(
                "Failed to terminate app: {}",
                error
            )));
        }

        Ok(())
    }

    pub fn get_app_container(&self, udid: &str, bundle_id: &str) -> Result<String> {
        let output = Command::new("xcrun")
            .args(["simctl", "get_app_container", udid, bundle_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to get app container: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to get app container: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn push_file(&self, udid: &str, local_path: &str, remote_path: &str) -> Result<()> {
        let container_path = if remote_path.starts_with('/') {
            remote_path.to_string()
        } else {
            format!("/tmp/{}", remote_path)
        };

        let output = Command::new("xcrun")
            .args([
                "simctl",
                "spawn",
                udid,
                "cp",
                "-f",
                local_path,
                &container_path,
            ])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to push file: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to push file: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    pub fn pull_file(&self, udid: &str, remote_path: &str, local_path: &str) -> Result<()> {
        let output = Command::new("xcrun")
            .args(["simctl", "spawn", udid, "cat", remote_path])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to pull file: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to pull file: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        std::fs::write(local_path, &output.stdout)
            .map_err(|e| TestError::Mcp(format!("Failed to write file: {}", e)))?;

        Ok(())
    }

    pub fn list_apps(&self, udid: &str) -> Result<Vec<AppInfo>> {
        let output = Command::new("xcrun")
            .args(["simctl", "listapps", udid])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list apps: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to list apps: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let apps: Vec<serde_json::Value> = serde_json::from_str(&json_str)
            .map_err(|e| TestError::Mcp(format!("Failed to parse app list: {}", e)))?;

        let mut app_list = Vec::new();
        for app in apps {
            if let (Some(bundle_id), Some(name)) = (
                app.get("CFBundleIdentifier").and_then(|b| b.as_str()),
                app.get("CFBundleName").and_then(|n| n.as_str()),
            ) {
                app_list.push(AppInfo {
                    bundle_id: bundle_id.to_string(),
                    name: name.to_string(),
                    path: app
                        .get("Path")
                        .and_then(|p| p.as_str())
                        .unwrap_or("")
                        .to_string(),
                    version: app
                        .get("CFBundleShortVersionString")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown")
                        .to_string(),
                });
            }
        }

        Ok(app_list)
    }

    pub fn take_screenshot(&self, udid: &str, output_path: &str) -> Result<()> {
        let output = Command::new("xcrun")
            .args(["simctl", "io", udid, "screenshot", output_path])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to take screenshot: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to take screenshot: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }

    pub fn record_video(&self, udid: &str, output_path: &str) -> Result<std::process::Child> {
        Command::new("xcrun")
            .args(["simctl", "io", udid, "recordVideo", output_path])
            .spawn()
            .map_err(|e| TestError::Mcp(format!("Failed to start video recording: {}", e)))
    }
}

impl Default for SimulatorManager {
    fn default() -> Self {
        Self::new()
    }
}
