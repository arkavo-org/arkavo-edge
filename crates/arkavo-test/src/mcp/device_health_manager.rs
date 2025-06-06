use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceHealth {
    pub device_id: String,
    pub device_name: String,
    pub is_healthy: bool,
    pub runtime_available: bool,
    pub runtime_id: String,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub identifier: String,
    pub version: String,
    pub is_available: bool,
    pub build_version: Option<String>,
}

pub struct DeviceHealthManager;

impl DeviceHealthManager {
    /// Get all available runtimes
    pub fn get_available_runtimes() -> Result<Vec<RuntimeInfo>> {
        let output = Command::new("xcrun")
            .args(["simctl", "list", "runtimes", "-j"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list runtimes: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to list runtimes: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let json_str = String::from_utf8_lossy(&output.stdout);
        let data: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| TestError::Mcp(format!("Failed to parse runtime list: {}", e)))?;

        let mut runtimes = Vec::new();
        
        if let Some(runtime_list) = data.get("runtimes").and_then(|r| r.as_array()) {
            for runtime in runtime_list {
                if let (Some(identifier), Some(version)) = (
                    runtime.get("identifier").and_then(|i| i.as_str()),
                    runtime.get("version").and_then(|v| v.as_str()),
                ) {
                    let is_available = runtime
                        .get("isAvailable")
                        .and_then(|a| a.as_bool())
                        .unwrap_or(false);
                    
                    let build_version = runtime
                        .get("buildversion")
                        .and_then(|b| b.as_str())
                        .map(|s| s.to_string());

                    runtimes.push(RuntimeInfo {
                        identifier: identifier.to_string(),
                        version: version.to_string(),
                        is_available,
                        build_version,
                    });
                }
            }
        }

        Ok(runtimes)
    }

    /// Check health of all devices
    pub fn check_all_devices_health() -> Result<Vec<DeviceHealth>> {
        // First, get available runtimes
        let available_runtimes = Self::get_available_runtimes()?;
        let available_runtime_ids: HashSet<String> = available_runtimes
            .iter()
            .filter(|r| r.is_available)
            .map(|r| r.identifier.clone())
            .collect();

        // Get all devices
        let output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "-j"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list devices: {}", e)))?;

        let json_str = String::from_utf8_lossy(&output.stdout);
        let data: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| TestError::Mcp(format!("Failed to parse device list: {}", e)))?;

        let mut health_reports = Vec::new();

        if let Some(devices_map) = data.get("devices").and_then(|d| d.as_object()) {
            for (runtime_id, device_list) in devices_map {
                if let Some(devices) = device_list.as_array() {
                    for device in devices {
                        if let (Some(udid), Some(name)) = (
                            device.get("udid").and_then(|u| u.as_str()),
                            device.get("name").and_then(|n| n.as_str()),
                        ) {
                            let mut issues = Vec::new();
                            
                            // Check if runtime is available
                            let runtime_available = available_runtime_ids.contains(runtime_id);
                            if !runtime_available {
                                issues.push(format!("Runtime {} is not available", runtime_id));
                            }
                            
                            // Check if device is available
                            let is_available = device
                                .get("isAvailable")
                                .and_then(|a| a.as_bool())
                                .unwrap_or(true); // Default to true for backwards compatibility
                            
                            if !is_available {
                                if let Some(error) = device.get("availabilityError").and_then(|e| e.as_str()) {
                                    issues.push(format!("Device unavailable: {}", error));
                                } else {
                                    issues.push("Device is marked as unavailable".to_string());
                                }
                            }

                            let is_healthy = runtime_available && is_available && issues.is_empty();

                            health_reports.push(DeviceHealth {
                                device_id: udid.to_string(),
                                device_name: name.to_string(),
                                is_healthy,
                                runtime_available,
                                runtime_id: runtime_id.clone(),
                                issues,
                            });
                        }
                    }
                }
            }
        }

        Ok(health_reports)
    }

    /// Delete unhealthy devices
    pub fn delete_unhealthy_devices(dry_run: bool) -> Result<Vec<String>> {
        let health_reports = Self::check_all_devices_health()?;
        let mut deleted_devices = Vec::new();

        for report in health_reports {
            if !report.is_healthy {
                if dry_run {
                    eprintln!(
                        "[DRY RUN] Would delete unhealthy device: {} ({}) - Issues: {:?}",
                        report.device_name, report.device_id, report.issues
                    );
                    deleted_devices.push(report.device_id);
                } else {
                    eprintln!(
                        "Deleting unhealthy device: {} ({}) - Issues: {:?}",
                        report.device_name, report.device_id, report.issues
                    );
                    
                    let output = Command::new("xcrun")
                        .args(["simctl", "delete", &report.device_id])
                        .output()
                        .map_err(|e| TestError::Mcp(format!("Failed to delete device: {}", e)))?;

                    if output.status.success() {
                        deleted_devices.push(report.device_id);
                    } else {
                        eprintln!(
                            "Failed to delete device {}: {}",
                            report.device_id,
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                }
            }
        }

        Ok(deleted_devices)
    }

    /// Delete all unavailable devices (simctl's built-in command)
    pub fn delete_unavailable_devices() -> Result<()> {
        eprintln!("Deleting all unavailable devices...");
        
        let output = Command::new("xcrun")
            .args(["simctl", "delete", "unavailable"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to delete unavailable devices: {}", e)))?;

        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to delete unavailable devices: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_health_serialization() {
        let health = DeviceHealth {
            device_id: "test-device".to_string(),
            device_name: "iPhone 15".to_string(),
            is_healthy: false,
            runtime_available: false,
            runtime_id: "com.apple.CoreSimulator.SimRuntime.iOS-17-0".to_string(),
            issues: vec!["Runtime not available".to_string()],
        };

        let json = serde_json::to_string_pretty(&health).unwrap();
        assert!(json.contains("\"is_healthy\": false"));
        assert!(json.contains("\"runtime_available\": false"));
    }
}