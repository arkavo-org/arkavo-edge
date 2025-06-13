use arkavo_idb_direct::{IdbDirect, IdbError, TargetType};
use once_cell::sync::Lazy;
use serde_json::json;
use std::sync::Mutex;

use crate::{Result, TestError};

// Global IDB Direct instance
static IDB_INSTANCE: Lazy<Mutex<Option<IdbDirect>>> = Lazy::new(|| Mutex::new(None));

// Track connected device
static CONNECTED_DEVICE: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Direct FFI wrapper for IDB functionality
pub struct IdbDirectWrapper;

impl IdbDirectWrapper {
    /// Initialize IDB Direct FFI
    pub fn initialize() -> Result<()> {
        eprintln!("[IdbDirectWrapper::initialize] Initializing IDB Direct FFI...");
        
        let mut instance_guard = IDB_INSTANCE.lock().unwrap();
        if instance_guard.is_none() {
            match IdbDirect::new() {
                Ok(idb) => {
                    eprintln!("[IdbDirectWrapper] IDB Direct initialized successfully");
                    eprintln!("[IdbDirectWrapper] Version: {}", IdbDirect::version());
                    *instance_guard = Some(idb);
                }
                Err(e) => {
                    eprintln!("[IdbDirectWrapper] Failed to initialize: {:?}", e);
                    return Err(TestError::Mcp(format!("IDB Direct initialization failed: {}", e)));
                }
            }
        }
        Ok(())
    }

    /// Connect to a target device
    pub fn connect_target(device_id: &str) -> Result<()> {
        eprintln!("[IdbDirectWrapper] Connecting to device: {}", device_id);
        
        let mut instance_guard = IDB_INSTANCE.lock().unwrap();
        let idb = instance_guard.as_mut()
            .ok_or_else(|| TestError::Mcp("IDB Direct not initialized".to_string()))?;
        
        // For now, assume all targets are simulators
        // In a real implementation, we'd detect the target type
        match idb.connect_target(device_id, TargetType::Simulator) {
            Ok(()) => {
                let mut device_guard = CONNECTED_DEVICE.lock().unwrap();
                *device_guard = Some(device_id.to_string());
                eprintln!("[IdbDirectWrapper] Connected to device: {}", device_id);
                Ok(())
            }
            Err(e) => {
                eprintln!("[IdbDirectWrapper] Failed to connect: {:?}", e);
                Err(TestError::Mcp(format!("Failed to connect to device: {}", e)))
            }
        }
    }

    /// Disconnect from current target
    pub fn disconnect_target() -> Result<()> {
        eprintln!("[IdbDirectWrapper] Disconnecting from device");
        
        let mut instance_guard = IDB_INSTANCE.lock().unwrap();
        if let Some(idb) = instance_guard.as_mut() {
            match idb.disconnect_target() {
                Ok(()) => {
                    let mut device_guard = CONNECTED_DEVICE.lock().unwrap();
                    *device_guard = None;
                    eprintln!("[IdbDirectWrapper] Disconnected successfully");
                    Ok(())
                }
                Err(e) => {
                    eprintln!("[IdbDirectWrapper] Failed to disconnect: {:?}", e);
                    Err(TestError::Mcp(format!("Failed to disconnect: {}", e)))
                }
            }
        } else {
            Ok(())
        }
    }

    /// Perform a tap at the specified coordinates
    pub fn tap(x: f64, y: f64) -> Result<serde_json::Value> {
        eprintln!("[IdbDirectWrapper] Tapping at ({}, {})", x, y);
        
        let instance_guard = IDB_INSTANCE.lock().unwrap();
        let idb = instance_guard.as_ref()
            .ok_or_else(|| TestError::Mcp("IDB Direct not initialized".to_string()))?;
        
        let start = std::time::Instant::now();
        match idb.tap(x, y) {
            Ok(()) => {
                let latency = start.elapsed().as_micros() as u64;
                eprintln!("[IdbDirectWrapper] Tap completed in {}μs", latency);
                
                Ok(json!({
                    "success": true,
                    "coordinates": {
                        "x": x,
                        "y": y
                    },
                    "latency_us": latency,
                    "method": "direct_ffi"
                }))
            }
            Err(e) => {
                eprintln!("[IdbDirectWrapper] Tap failed: {:?}", e);
                Err(TestError::Mcp(format!("Tap failed: {}", e)))
            }
        }
    }

    /// Take a screenshot
    pub async fn take_screenshot() -> Result<Vec<u8>> {
        eprintln!("[IdbDirectWrapper] Taking screenshot");
        
        let instance_guard = IDB_INSTANCE.lock().unwrap();
        let idb = instance_guard.as_ref()
            .ok_or_else(|| TestError::Mcp("IDB Direct not initialized".to_string()))?;
        
        match idb.take_screenshot() {
            Ok(screenshot) => {
                eprintln!(
                    "[IdbDirectWrapper] Screenshot captured: {}x{} ({} format)",
                    screenshot.width, screenshot.height, screenshot.format
                );
                Ok(screenshot.data().to_vec())
            }
            Err(e) => {
                eprintln!("[IdbDirectWrapper] Screenshot failed: {:?}", e);
                Err(TestError::Mcp(format!("Screenshot failed: {}", e)))
            }
        }
    }

    /// List available targets
    pub fn list_targets() -> Result<Vec<serde_json::Value>> {
        eprintln!("[IdbDirectWrapper] Listing targets");
        
        let instance_guard = IDB_INSTANCE.lock().unwrap();
        let idb = instance_guard.as_ref()
            .ok_or_else(|| TestError::Mcp("IDB Direct not initialized".to_string()))?;
        
        match idb.list_targets() {
            Ok(targets) => {
                let result: Vec<serde_json::Value> = targets.into_iter()
                    .map(|t| json!({
                        "udid": t.udid,
                        "name": t.name,
                        "os_version": t.os_version,
                        "device_type": t.device_type,
                        "is_running": t.is_running,
                        "target_type": match t.target_type {
                            TargetType::Simulator => "simulator",
                            TargetType::Device => "device",
                        }
                    }))
                    .collect();
                
                eprintln!("[IdbDirectWrapper] Found {} targets", result.len());
                Ok(result)
            }
            Err(e) => {
                eprintln!("[IdbDirectWrapper] Failed to list targets: {:?}", e);
                Err(TestError::Mcp(format!("Failed to list targets: {}", e)))
            }
        }
    }

    /// Check if IDB Direct is available
    pub fn is_available() -> bool {
        #[cfg(target_os = "macos")]
        {
            // Check if the static library exists
            let manifest_dir = env!("CARGO_MANIFEST_DIR");
            let lib_path = std::path::Path::new(manifest_dir)
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("vendor")
                .join("idb")
                .join("libidb_direct.a");
            
            lib_path.exists()
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    /// Get version information
    pub fn version() -> String {
        IdbDirect::version().to_string()
    }
}