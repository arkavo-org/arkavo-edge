use super::device_manager::{DeviceManager, DeviceState, IOSDevice};
use super::xctest_verifier::{XCTestStatus, XCTestVerifier};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceWithXCTestStatus {
    pub device: IOSDevice,
    pub xctest_status: Option<XCTestStatus>,
}

pub struct DeviceXCTestStatusManager;

impl DeviceXCTestStatusManager {
    /// Get all devices with their XCTest status
    pub async fn get_all_devices_with_status(
        device_manager: Arc<DeviceManager>,
    ) -> Result<Vec<DeviceWithXCTestStatus>> {
        let devices = device_manager.refresh_devices()?;
        let mut devices_with_status = Vec::new();
        
        for device in devices {
            let xctest_status = if device.state == DeviceState::Booted {
                // Only check XCTest status for booted devices
                match XCTestVerifier::verify_device(&device.id).await {
                    Ok(status) => Some(status),
                    Err(e) => {
                        eprintln!("Failed to verify XCTest for device {}: {}", device.id, e);
                        None
                    }
                }
            } else {
                None
            };
            
            devices_with_status.push(DeviceWithXCTestStatus {
                device,
                xctest_status,
            });
        }
        
        Ok(devices_with_status)
    }
    
    /// Find the best device for XCTest operations
    pub async fn find_best_xctest_device(
        device_manager: Arc<DeviceManager>,
    ) -> Result<Option<DeviceWithXCTestStatus>> {
        let devices = Self::get_all_devices_with_status(device_manager).await?;
        
        // Priority order:
        // 1. Booted device with functional XCTest
        // 2. Booted device with XCTest installed but not functional
        // 3. Booted device without XCTest
        // 4. Shutdown device (would need to be booted first)
        
        let mut best_device: Option<DeviceWithXCTestStatus> = None;
        let mut best_score = 0;
        
        for device in devices {
            let score = match (&device.device.state, &device.xctest_status) {
                (DeviceState::Booted, Some(status)) if status.is_functional => 100,
                (DeviceState::Booted, Some(status)) if status.bundle_installed => 75,
                (DeviceState::Booted, Some(_)) => 50,
                (DeviceState::Booted, None) => 25,
                (DeviceState::Shutdown, _) => 10,
                _ => 0,
            };
            
            if score > best_score {
                best_score = score;
                best_device = Some(device);
            }
        }
        
        Ok(best_device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_device_with_xctest_status_serialization() {
        use std::time::Duration;
        
        let device_status = DeviceWithXCTestStatus {
            device: IOSDevice {
                id: "device-123".to_string(),
                name: "iPhone 15".to_string(),
                device_type: "iPhone 15".to_string(),
                runtime: "iOS 17.0".to_string(),
                state: DeviceState::Booted,
                is_physical: false,
            },
            xctest_status: Some(XCTestStatus {
                device_id: "device-123".to_string(),
                is_functional: true,
                bundle_installed: true,
                bridge_connectable: true,
                swift_response_time: Some(Duration::from_millis(200)),
                error_details: None,
            }),
        };
        
        let json = serde_json::to_string_pretty(&device_status).unwrap();
        println!("Device with XCTest status JSON:\n{}", json);
        
        let parsed: DeviceWithXCTestStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.device.id, "device-123");
        assert!(parsed.xctest_status.is_some());
        assert!(parsed.xctest_status.unwrap().is_functional);
    }
}