use super::device_manager::DeviceManager;
use super::device_xctest_status::DeviceXCTestStatusManager;
use super::server::{Tool, ToolSchema};
use crate::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct XCTestStatusKit {
    schema: ToolSchema,
    device_manager: Arc<DeviceManager>,
}

impl XCTestStatusKit {
    pub fn new(device_manager: Arc<DeviceManager>) -> Self {
        Self {
            schema: ToolSchema {
                name: "xctest_status".to_string(),
                description: "Check XCTest bridge functionality status for all devices or a specific device. Returns detailed information about which devices have functional XCTest support.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "device_id": {
                            "type": "string",
                            "description": "Optional specific device ID to check. If not provided, checks all devices."
                        },
                        "find_best": {
                            "type": "boolean",
                            "description": "If true, returns only the best device for XCTest operations",
                            "default": false
                        }
                    }
                }),
            },
            device_manager,
        }
    }
}

#[async_trait]
impl Tool for XCTestStatusKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let device_id = params.get("device_id").and_then(|v| v.as_str());
        let find_best = params
            .get("find_best")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if find_best {
            // Find the best device for XCTest
            match DeviceXCTestStatusManager::find_best_xctest_device(self.device_manager.clone())
                .await
            {
                Ok(Some(best_device)) => Ok(serde_json::json!({
                    "success": true,
                    "best_device": best_device,
                    "recommendation": if best_device.xctest_status.as_ref().map(|s| s.is_functional).unwrap_or(false) {
                        "This device is ready for XCTest operations"
                    } else if best_device.xctest_status.as_ref().map(|s| s.bundle_installed).unwrap_or(false) {
                        "XCTest bundle is installed but not functional. Run setup_xcuitest to fix."
                    } else {
                        "No XCTest bundle installed. Run setup_xcuitest to install."
                    }
                })),
                Ok(None) => Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "NO_DEVICES",
                        "message": "No iOS devices found"
                    }
                })),
                Err(e) => Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "STATUS_CHECK_FAILED",
                        "message": format!("Failed to check device status: {}", e)
                    }
                })),
            }
        } else if let Some(device_id) = device_id {
            // Check specific device
            use super::xctest_verifier::XCTestVerifier;

            match XCTestVerifier::verify_device(device_id).await {
                Ok(status) => Ok(serde_json::json!({
                    "success": true,
                    "device_id": device_id,
                    "xctest_status": status,
                    "summary": {
                        "functional": status.is_functional,
                        "bundle_installed": status.bundle_installed,
                        "bridge_works": status.bridge_connectable,
                        "response_time_ms": status.swift_response_time.map(|d| d.as_millis()),
                        "error": status.error_details
                    }
                })),
                Err(e) => Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "VERIFICATION_FAILED",
                        "message": format!("Failed to verify device: {}", e),
                        "device_id": device_id
                    }
                })),
            }
        } else {
            // Check all devices
            match DeviceXCTestStatusManager::get_all_devices_with_status(
                self.device_manager.clone(),
            )
            .await
            {
                Ok(devices) => {
                    let functional_count = devices
                        .iter()
                        .filter(|d| {
                            d.xctest_status
                                .as_ref()
                                .map(|s| s.is_functional)
                                .unwrap_or(false)
                        })
                        .count();

                    let with_bundle_count = devices
                        .iter()
                        .filter(|d| {
                            d.xctest_status
                                .as_ref()
                                .map(|s| s.bundle_installed)
                                .unwrap_or(false)
                        })
                        .count();

                    let booted_count = devices
                        .iter()
                        .filter(|d| d.device.state == super::device_manager::DeviceState::Booted)
                        .count();

                    Ok(serde_json::json!({
                        "success": true,
                        "summary": {
                            "total_devices": devices.len(),
                            "booted_devices": booted_count,
                            "xctest_functional": functional_count,
                            "xctest_installed": with_bundle_count
                        },
                        "devices": devices,
                        "recommendations": {
                            "has_functional": functional_count > 0,
                            "next_steps": if functional_count > 0 {
                                "XCTest is ready on one or more devices"
                            } else if with_bundle_count > 0 {
                                "XCTest bundle installed but not functional. Run setup_xcuitest with force_reinstall:true"
                            } else if booted_count > 0 {
                                "Booted devices available. Run setup_xcuitest to install XCTest"
                            } else {
                                "No booted devices. Boot a simulator first with device_management tool"
                            }
                        }
                    }))
                }
                Err(e) => Ok(serde_json::json!({
                    "success": false,
                    "error": {
                        "code": "STATUS_CHECK_FAILED",
                        "message": format!("Failed to check device status: {}", e)
                    }
                })),
            }
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
