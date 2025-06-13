use serde_json::json;
use std::sync::Mutex;
use once_cell::sync::Lazy;

use crate::{Result, TestError};

#[cfg(target_os = "macos")]
use super::idb_direct_wrapper::IdbDirectWrapper;
#[cfg(target_os = "macos")]
use super::idb_wrapper::IdbWrapper;

// Track which backend is in use
static BACKEND: Lazy<Mutex<Backend>> = Lazy::new(|| Mutex::new(Backend::Auto));

#[derive(Debug, Clone, Copy)]
enum Backend {
    Auto,      // Automatically choose based on availability
    Direct,    // Force Direct FFI
    Companion, // Force IDB Companion
}

/// Unified IDB interface that can use either Direct FFI or IDB Companion
pub struct IdbUnified;

impl IdbUnified {
    /// Initialize IDB with automatic backend selection
    pub fn initialize() -> Result<()> {
        Self::initialize_with_backend(Backend::Auto)
    }

    /// Initialize with specific backend preference
    pub fn initialize_with_backend(backend: Backend) -> Result<()> {
        eprintln!("[IdbUnified::initialize] Initializing with backend: {:?}", backend);
        
        #[cfg(not(target_os = "macos"))]
        {
            return Err(TestError::Mcp("IDB is only supported on macOS".to_string()));
        }
        
        #[cfg(target_os = "macos")]
        {
            let mut backend_guard = BACKEND.lock().unwrap();
            
            match backend {
                Backend::Direct => {
                    if IdbDirectWrapper::is_available() {
                        eprintln!("[IdbUnified] Using Direct FFI backend");
                        *backend_guard = Backend::Direct;
                        return IdbDirectWrapper::initialize();
                    } else {
                        return Err(TestError::Mcp(
                            "Direct FFI backend requested but libidb_direct.a not found".to_string()
                        ));
                    }
                }
                Backend::Companion => {
                    eprintln!("[IdbUnified] Using IDB Companion backend");
                    *backend_guard = Backend::Companion;
                    return IdbWrapper::initialize();
                }
                Backend::Auto => {
                    // Try Direct FFI first if available
                    if IdbDirectWrapper::is_available() {
                        eprintln!("[IdbUnified] Auto-selected Direct FFI backend");
                        match IdbDirectWrapper::initialize() {
                            Ok(()) => {
                                *backend_guard = Backend::Direct;
                                return Ok(());
                            }
                            Err(e) => {
                                eprintln!("[IdbUnified] Direct FFI failed: {}, falling back to companion", e);
                            }
                        }
                    }
                    
                    // Fall back to IDB Companion
                    eprintln!("[IdbUnified] Auto-selected IDB Companion backend");
                    *backend_guard = Backend::Companion;
                    return IdbWrapper::initialize();
                }
            }
        }
    }

    /// Connect to a target device
    pub fn connect_target(device_id: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let backend = *BACKEND.lock().unwrap();
            match backend {
                Backend::Direct => IdbDirectWrapper::connect_target(device_id),
                Backend::Companion | Backend::Auto => {
                    // IdbWrapper doesn't have explicit connect, it's done per-command
                    Ok(())
                }
            }
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            Err(TestError::Mcp("IDB is only supported on macOS".to_string()))
        }
    }

    /// Perform a tap at specified coordinates
    pub async fn tap(device_id: &str, x: f64, y: f64) -> Result<serde_json::Value> {
        #[cfg(target_os = "macos")]
        {
            let backend = *BACKEND.lock().unwrap();
            match backend {
                Backend::Direct => {
                    // Ensure we're connected to the right device
                    IdbDirectWrapper::connect_target(device_id)?;
                    IdbDirectWrapper::tap(x, y)
                }
                Backend::Companion | Backend::Auto => {
                    // Use the existing IDB companion tap command
                    let idb_path = IdbWrapper::get_binary_path()?;
                    let output = std::process::Command::new(&idb_path)
                        .args(&[
                            "ui", "tap",
                            "--udid", device_id,
                            &x.to_string(), &y.to_string()
                        ])
                        .output()
                        .map_err(|e| TestError::Mcp(format!("Failed to execute tap: {}", e)))?;
                    
                    if output.status.success() {
                        Ok(json!({
                            "success": true,
                            "coordinates": {
                                "x": x,
                                "y": y
                            },
                            "method": "companion"
                        }))
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        Err(TestError::Mcp(format!("Tap failed: {}", stderr)))
                    }
                }
            }
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            Err(TestError::Mcp("IDB is only supported on macOS".to_string()))
        }
    }

    /// Take a screenshot
    pub async fn take_screenshot(device_id: &str) -> Result<Vec<u8>> {
        #[cfg(target_os = "macos")]
        {
            let backend = *BACKEND.lock().unwrap();
            match backend {
                Backend::Direct => {
                    // Ensure we're connected to the right device
                    IdbDirectWrapper::connect_target(device_id)?;
                    IdbDirectWrapper::take_screenshot().await
                }
                Backend::Companion | Backend::Auto => {
                    // Use the existing IDB companion screenshot command
                    let idb_path = IdbWrapper::get_binary_path()?;
                    let output = std::process::Command::new(&idb_path)
                        .args(&[
                            "screenshot",
                            "--udid", device_id,
                            "--format", "png"
                        ])
                        .output()
                        .map_err(|e| TestError::Mcp(format!("Failed to take screenshot: {}", e)))?;
                    
                    if output.status.success() {
                        Ok(output.stdout)
                    } else {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        Err(TestError::Mcp(format!("Screenshot failed: {}", stderr)))
                    }
                }
            }
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            Err(TestError::Mcp("IDB is only supported on macOS".to_string()))
        }
    }

    /// Get backend information
    pub fn get_backend_info() -> serde_json::Value {
        #[cfg(target_os = "macos")]
        {
            let backend = *BACKEND.lock().unwrap();
            json!({
                "backend": match backend {
                    Backend::Direct => "direct_ffi",
                    Backend::Companion => "companion",
                    Backend::Auto => "auto",
                },
                "direct_available": IdbDirectWrapper::is_available(),
                "version": match backend {
                    Backend::Direct => IdbDirectWrapper::version(),
                    _ => "companion".to_string(),
                }
            })
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            json!({
                "backend": "none",
                "error": "IDB is only supported on macOS"
            })
        }
    }
}