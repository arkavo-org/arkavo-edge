#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

// pub mod shm; // TODO: Enable once bindings are properly generated

use std::ffi::{CStr, CString};
use std::ptr;
use thiserror::Error;

// Manual constants until bindgen issue is resolved
const IDB_SUCCESS: idb_error_t = 0;
const IDB_ERROR_NOT_INITIALIZED: idb_error_t = -1;
const IDB_ERROR_INVALID_PARAMETER: idb_error_t = -2;
const IDB_ERROR_DEVICE_NOT_FOUND: idb_error_t = -3;
const IDB_ERROR_SIMULATOR_NOT_RUNNING: idb_error_t = -4;
const IDB_ERROR_OPERATION_FAILED: idb_error_t = -5;
const IDB_ERROR_TIMEOUT: idb_error_t = -6;
const IDB_ERROR_OUT_OF_MEMORY: idb_error_t = -7;
const IDB_ERROR_NOT_IMPLEMENTED: idb_error_t = -100;
const IDB_ERROR_UNSUPPORTED: idb_error_t = -101;
const IDB_ERROR_PERMISSION_DENIED: idb_error_t = -102;
const IDB_ERROR_APP_NOT_FOUND: idb_error_t = -103;
const IDB_ERROR_INVALID_APP_BUNDLE: idb_error_t = -104;

const IDB_TARGET_SIMULATOR: idb_target_type_t = 0;
const IDB_TARGET_DEVICE: idb_target_type_t = 1;

#[derive(Error, Debug)]
pub enum IdbError {
    #[error("IDB not initialized")]
    NotInitialized,
    #[error("Invalid parameter")]
    InvalidParameter,
    #[error("Device not found")]
    DeviceNotFound,
    #[error("Simulator not running")]
    SimulatorNotRunning,
    #[error("Operation failed")]
    OperationFailed,
    #[error("Timeout")]
    Timeout,
    #[error("Out of memory")]
    OutOfMemory,
    #[error("Not implemented")]
    NotImplemented,
    #[error("Unsupported")]
    Unsupported,
    #[error("Permission denied")]
    PermissionDenied,
    #[error("App not found")]
    AppNotFound,
    #[error("Invalid app bundle")]
    InvalidAppBundle,
    #[error("Unknown error: {0}")]
    Unknown(i32),
}

impl From<idb_error_t> for IdbError {
    fn from(err: idb_error_t) -> Self {
        match err {
            IDB_ERROR_NOT_INITIALIZED => IdbError::NotInitialized,
            IDB_ERROR_INVALID_PARAMETER => IdbError::InvalidParameter,
            IDB_ERROR_DEVICE_NOT_FOUND => IdbError::DeviceNotFound,
            IDB_ERROR_SIMULATOR_NOT_RUNNING => IdbError::SimulatorNotRunning,
            IDB_ERROR_OPERATION_FAILED => IdbError::OperationFailed,
            IDB_ERROR_TIMEOUT => IdbError::Timeout,
            IDB_ERROR_OUT_OF_MEMORY => IdbError::OutOfMemory,
            IDB_ERROR_NOT_IMPLEMENTED => IdbError::NotImplemented,
            IDB_ERROR_UNSUPPORTED => IdbError::Unsupported,
            IDB_ERROR_PERMISSION_DENIED => IdbError::PermissionDenied,
            IDB_ERROR_APP_NOT_FOUND => IdbError::AppNotFound,
            IDB_ERROR_INVALID_APP_BUNDLE => IdbError::InvalidAppBundle,
            _ => IdbError::Unknown(err),
        }
    }
}

type Result<T> = std::result::Result<T, IdbError>;

#[derive(Debug)]
pub struct IdbDirect {
    connected: bool,
}

impl IdbDirect {
    pub fn new() -> Result<Self> {
        unsafe {
            let err = idb_initialize();
            if err != IDB_SUCCESS {
                return Err(err.into());
            }
        }
        Ok(Self { connected: false })
    }

    pub fn connect_target(&mut self, udid: &str, target_type: TargetType) -> Result<()> {
        let c_udid = CString::new(udid).map_err(|_| IdbError::InvalidParameter)?;
        unsafe {
            let err = idb_connect_target(c_udid.as_ptr(), target_type.into());
            if err != IDB_SUCCESS {
                return Err(err.into());
            }
        }
        self.connected = true;
        Ok(())
    }

    pub fn disconnect_target(&mut self) -> Result<()> {
        unsafe {
            let err = idb_disconnect_target();
            if err != IDB_SUCCESS {
                return Err(err.into());
            }
        }
        self.connected = false;
        Ok(())
    }

    pub fn tap(&self, x: f64, y: f64) -> Result<()> {
        if !self.connected {
            return Err(IdbError::NotInitialized);
        }
        unsafe {
            let err = idb_tap(x, y);
            if err != IDB_SUCCESS {
                return Err(err.into());
            }
        }
        Ok(())
    }

    pub fn take_screenshot(&self) -> Result<Screenshot> {
        if !self.connected {
            return Err(IdbError::NotInitialized);
        }
        
        let mut screenshot = idb_screenshot_t {
            data: ptr::null_mut(),
            size: 0,
            width: 0,
            height: 0,
            format: ptr::null_mut(),
        };
        
        unsafe {
            let err = idb_take_screenshot(&mut screenshot);
            if err != IDB_SUCCESS {
                return Err(err.into());
            }
        }
        
        Ok(Screenshot::from_raw(screenshot))
    }

    pub fn list_targets(&self) -> Result<Vec<TargetInfo>> {
        // TODO: Enable once idb_list_targets is implemented in the static library
        eprintln!("[IdbDirect] list_targets not yet implemented in static library");
        Err(IdbError::NotImplemented)
        
        // let mut targets: *mut idb_target_info_t = ptr::null_mut();
        // let mut count: usize = 0;
        // 
        // unsafe {
        //     let err = idb_list_targets(&mut targets, &mut count);
        //     if err != IDB_SUCCESS {
        //         return Err(err.into());
        //     }
        //     
        //     let mut result = Vec::with_capacity(count);
        //     for i in 0..count {
        //         let target = &*targets.add(i);
        //         result.push(TargetInfo::from_raw(target));
        //     }
        //     
        //     idb_free_targets(targets, count);
        //     Ok(result)
        // }
    }

    pub fn version() -> &'static str {
        unsafe {
            CStr::from_ptr(idb_version())
                .to_str()
                .unwrap_or("unknown")
        }
    }
}

impl Drop for IdbDirect {
    fn drop(&mut self) {
        if self.connected {
            let _ = self.disconnect_target();
        }
        unsafe {
            let _ = idb_shutdown();
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TargetType {
    Simulator,
    Device,
}

impl From<TargetType> for idb_target_type_t {
    fn from(t: TargetType) -> Self {
        match t {
            TargetType::Simulator => IDB_TARGET_SIMULATOR,
            TargetType::Device => IDB_TARGET_DEVICE,
        }
    }
}

pub struct Screenshot {
    data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: String,
}

impl Screenshot {
    fn from_raw(raw: idb_screenshot_t) -> Self {
        let data = unsafe {
            std::slice::from_raw_parts(raw.data, raw.size).to_vec()
        };
        
        let format = unsafe {
            CStr::from_ptr(raw.format)
                .to_str()
                .unwrap_or("png")
                .to_string()
        };
        
        let screenshot = Self {
            data,
            width: raw.width,
            height: raw.height,
            format,
        };
        
        unsafe {
            idb_free_screenshot(&raw as *const _ as *mut _);
        }
        
        screenshot
    }
    
    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub udid: String,
    pub name: String,
    pub os_version: String,
    pub device_type: String,
    pub target_type: TargetType,
    pub is_running: bool,
}

impl TargetInfo {
    fn from_raw(raw: &idb_target_info_t) -> Self {
        Self {
            udid: unsafe { CStr::from_ptr(raw.udid).to_str().unwrap_or("").to_string() },
            name: unsafe { CStr::from_ptr(raw.name).to_str().unwrap_or("").to_string() },
            os_version: unsafe { CStr::from_ptr(raw.os_version).to_str().unwrap_or("").to_string() },
            device_type: unsafe { CStr::from_ptr(raw.device_type).to_str().unwrap_or("").to_string() },
            target_type: match raw.type_ {
                IDB_TARGET_SIMULATOR => TargetType::Simulator,
                _ => TargetType::Device,
            },
            is_running: raw.is_running,
        }
    }
}