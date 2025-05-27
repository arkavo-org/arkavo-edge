use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IOSDevice {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub runtime: String,
    pub state: DeviceState,
    pub is_physical: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DeviceState {
    Shutdown,
    Booted,
    Creating,
    Booting,
    ShuttingDown,
}

pub struct DeviceManager {
    devices: Arc<Mutex<HashMap<String, IOSDevice>>>,
    active_device_id: Arc<Mutex<Option<String>>>,
}

impl DeviceManager {
    pub fn new() -> Self {
        let manager = Self {
            devices: Arc::new(Mutex::new(HashMap::new())),
            active_device_id: Arc::new(Mutex::new(None)),
        };
        
        // Refresh device list on initialization
        let _ = manager.refresh_devices();
        manager
    }
    
    pub fn refresh_devices(&self) -> Result<Vec<IOSDevice>> {
        // Get simulators from simctl
        let output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "-j"])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to list devices: {}", e)))?;
        
        let json_str = String::from_utf8_lossy(&output.stdout);
        let parsed: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| TestError::Mcp(format!("Failed to parse device list: {}", e)))?;
        
        let mut devices = Vec::new();
        let mut device_map = self.devices.lock().unwrap();
        device_map.clear();
        
        // Parse simulators
        if let Some(device_obj) = parsed.get("devices").and_then(|d| d.as_object()) {
            for (runtime, device_list) in device_obj {
                if let Some(devices_array) = device_list.as_array() {
                    for device_json in devices_array {
                        if let Some(device) = self.parse_device(device_json, runtime, false) {
                            device_map.insert(device.id.clone(), device.clone());
                            devices.push(device);
                        }
                    }
                }
            }
        }
        
        // Also check for physical devices (via devicectl or idevice_id)
        if let Ok(physical_devices) = self.list_physical_devices() {
            for device in physical_devices {
                device_map.insert(device.id.clone(), device.clone());
                devices.push(device);
            }
        }
        
        // Set active device if none is set and we have booted devices
        if self.active_device_id.lock().unwrap().is_none() {
            if let Some(booted_device) = devices.iter().find(|d| d.state == DeviceState::Booted) {
                *self.active_device_id.lock().unwrap() = Some(booted_device.id.clone());
            }
        }
        
        Ok(devices)
    }
    
    fn parse_device(&self, device_json: &serde_json::Value, runtime: &str, is_physical: bool) -> Option<IOSDevice> {
        let id = device_json.get("udid")?.as_str()?;
        let name = device_json.get("name")?.as_str()?;
        let device_type = device_json.get("deviceTypeIdentifier")
            .and_then(|d| d.as_str())
            .unwrap_or("Unknown");
        
        let state_str = device_json.get("state")?.as_str()?;
        let state = match state_str {
            "Shutdown" => DeviceState::Shutdown,
            "Booted" => DeviceState::Booted,
            "Creating" => DeviceState::Creating,
            "Booting" => DeviceState::Booting,
            "ShuttingDown" => DeviceState::ShuttingDown,
            _ => return None,
        };
        
        Some(IOSDevice {
            id: id.to_string(),
            name: name.to_string(),
            device_type: device_type.to_string(),
            runtime: runtime.to_string(),
            state,
            is_physical,
        })
    }
    
    fn list_physical_devices(&self) -> Result<Vec<IOSDevice>> {
        // Try using idevice_id if available
        if let Ok(output) = Command::new("idevice_id").arg("-l").output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let devices: Vec<IOSDevice> = stdout
                    .lines()
                    .filter(|line| !line.is_empty())
                    .enumerate()
                    .map(|(i, udid)| IOSDevice {
                        id: udid.to_string(),
                        name: format!("Physical Device {}", i + 1),
                        device_type: "Physical".to_string(),
                        runtime: "iOS".to_string(),
                        state: DeviceState::Booted,
                        is_physical: true,
                    })
                    .collect();
                return Ok(devices);
            }
        }
        
        // TODO: Try devicectl for newer Xcode versions
        Ok(vec![])
    }
    
    pub fn get_device(&self, device_id: &str) -> Option<IOSDevice> {
        self.devices.lock().unwrap().get(device_id).cloned()
    }
    
    pub fn get_active_device(&self) -> Option<IOSDevice> {
        let device_id = self.active_device_id.lock().unwrap().clone()?;
        self.get_device(&device_id)
    }
    
    pub fn set_active_device(&self, device_id: &str) -> Result<()> {
        let devices = self.devices.lock().unwrap();
        if !devices.contains_key(device_id) {
            return Err(TestError::Mcp(format!("Device not found: {}", device_id)));
        }
        
        *self.active_device_id.lock().unwrap() = Some(device_id.to_string());
        Ok(())
    }
    
    pub fn get_all_devices(&self) -> Vec<IOSDevice> {
        self.devices.lock().unwrap().values().cloned().collect()
    }
    
    pub fn get_booted_devices(&self) -> Vec<IOSDevice> {
        self.devices
            .lock()
            .unwrap()
            .values()
            .filter(|d| d.state == DeviceState::Booted)
            .cloned()
            .collect()
    }
    
    pub fn boot_device(&self, device_id: &str) -> Result<()> {
        let device = self.get_device(device_id)
            .ok_or_else(|| TestError::Mcp(format!("Device not found: {}", device_id)))?;
        
        if device.is_physical {
            return Err(TestError::Mcp("Cannot boot physical device".to_string()));
        }
        
        Command::new("xcrun")
            .args(["simctl", "boot", device_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to boot device: {}", e)))?;
        
        // Update device state
        if let Some(device) = self.devices.lock().unwrap().get_mut(device_id) {
            device.state = DeviceState::Booted;
        }
        
        Ok(())
    }
    
    pub fn shutdown_device(&self, device_id: &str) -> Result<()> {
        let device = self.get_device(device_id)
            .ok_or_else(|| TestError::Mcp(format!("Device not found: {}", device_id)))?;
        
        if device.is_physical {
            return Err(TestError::Mcp("Cannot shutdown physical device".to_string()));
        }
        
        Command::new("xcrun")
            .args(["simctl", "shutdown", device_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to shutdown device: {}", e)))?;
        
        // Update device state
        if let Some(device) = self.devices.lock().unwrap().get_mut(device_id) {
            device.state = DeviceState::Shutdown;
        }
        
        Ok(())
    }
    
    pub fn create_device(&self, name: &str, device_type: &str, runtime: &str) -> Result<String> {
        let output = Command::new("xcrun")
            .args(["simctl", "create", name, device_type, runtime])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to create device: {}", e)))?;
        
        if !output.status.success() {
            return Err(TestError::Mcp(format!(
                "Failed to create device: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        
        let device_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        
        // Refresh devices to include the new one
        self.refresh_devices()?;
        
        Ok(device_id)
    }
    
    pub fn delete_device(&self, device_id: &str) -> Result<()> {
        Command::new("xcrun")
            .args(["simctl", "delete", device_id])
            .output()
            .map_err(|e| TestError::Mcp(format!("Failed to delete device: {}", e)))?;
        
        // Remove from our cache
        self.devices.lock().unwrap().remove(device_id);
        
        // Clear active device if it was deleted
        if self.active_device_id.lock().unwrap().as_ref() == Some(&device_id.to_string()) {
            *self.active_device_id.lock().unwrap() = None;
        }
        
        Ok(())
    }
}

impl Default for DeviceManager {
    fn default() -> Self {
        Self::new()
    }
}