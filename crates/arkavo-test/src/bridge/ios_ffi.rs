use crate::{Result, TestError};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};

#[repr(C)]
pub struct IOSBridge {
    _private: [u8; 0],
}

#[derive(Debug)]
pub struct RustTestHarness {
    bridge: Option<*mut IOSBridge>,
    snapshots: HashMap<String, Vec<u8>>,
}

impl RustTestHarness {
    pub fn new() -> Self {
        Self {
            bridge: None,
            snapshots: HashMap::new(),
        }
    }

    pub fn connect_ios_bridge(&mut self, bridge: *mut IOSBridge) {
        self.bridge = Some(bridge);
    }

    pub fn execute_action(&self, action: &str, params: &str) -> Result<String> {
        if self.bridge.is_none() {
            return Ok(serde_json::json!({
                "status": "simulated",
                "message": "iOS bridge not connected"
            })
            .to_string());
        }

        let action_cstr = CString::new(action)
            .map_err(|e| TestError::Bridge(format!("Invalid action string: {}", e)))?;
        let params_cstr = CString::new(params)
            .map_err(|e| TestError::Bridge(format!("Invalid params string: {}", e)))?;

        unsafe {
            let result_ptr = ios_bridge_execute_action(
                self.bridge.unwrap(),
                action_cstr.as_ptr(),
                params_cstr.as_ptr(),
            );

            if result_ptr.is_null() {
                return Err(TestError::Bridge("Null result from iOS bridge".to_string()));
            }

            let result_cstr = CStr::from_ptr(result_ptr);
            let result = result_cstr.to_string_lossy().to_string();

            ios_bridge_free_string(result_ptr);

            Ok(result)
        }
    }

    pub fn get_current_state(&self) -> Result<String> {
        if self.bridge.is_none() {
            return Ok(serde_json::json!({
                "status": "simulated",
                "state": {}
            })
            .to_string());
        }

        unsafe {
            let state_ptr = ios_bridge_get_current_state(self.bridge.unwrap());

            if state_ptr.is_null() {
                return Err(TestError::Bridge("Null state from iOS bridge".to_string()));
            }

            let state_cstr = CStr::from_ptr(state_ptr);
            let state = state_cstr.to_string_lossy().to_string();

            ios_bridge_free_string(state_ptr);

            Ok(state)
        }
    }

    pub fn mutate_state(&self, entity: &str, action: &str, data: &str) -> Result<String> {
        if self.bridge.is_none() {
            return Ok(serde_json::json!({
                "status": "simulated",
                "success": true
            })
            .to_string());
        }

        let entity_cstr = CString::new(entity)
            .map_err(|e| TestError::Bridge(format!("Invalid entity string: {}", e)))?;
        let action_cstr = CString::new(action)
            .map_err(|e| TestError::Bridge(format!("Invalid action string: {}", e)))?;
        let data_cstr = CString::new(data)
            .map_err(|e| TestError::Bridge(format!("Invalid data string: {}", e)))?;

        unsafe {
            let result_ptr = ios_bridge_mutate_state(
                self.bridge.unwrap(),
                entity_cstr.as_ptr(),
                action_cstr.as_ptr(),
                data_cstr.as_ptr(),
            );

            if result_ptr.is_null() {
                return Err(TestError::Bridge("Null result from iOS bridge".to_string()));
            }

            let result_cstr = CStr::from_ptr(result_ptr);
            let result = result_cstr.to_string_lossy().to_string();

            ios_bridge_free_string(result_ptr);

            Ok(result)
        }
    }

    pub fn checkpoint(&mut self, name: &str) -> Result<()> {
        let snapshot = self.create_snapshot()?;
        self.snapshots.insert(name.to_string(), snapshot);
        Ok(())
    }

    pub fn restore(&mut self, name: &str) -> Result<()> {
        let snapshot = self
            .snapshots
            .get(name)
            .ok_or_else(|| TestError::Bridge(format!("Snapshot not found: {}", name)))?
            .clone();

        self.restore_snapshot(&snapshot)?;
        Ok(())
    }

    pub fn branch(&mut self, from: &str, to: &str) -> Result<()> {
        let snapshot = self
            .snapshots
            .get(from)
            .ok_or_else(|| TestError::Bridge(format!("Parent snapshot not found: {}", from)))?
            .clone();

        self.snapshots.insert(to.to_string(), snapshot);
        Ok(())
    }

    fn create_snapshot(&self) -> Result<Vec<u8>> {
        if self.bridge.is_none() {
            return Ok(vec![]);
        }

        unsafe {
            let mut size: usize = 0;
            let snapshot_ptr = ios_bridge_create_snapshot(self.bridge.unwrap(), &mut size);

            if snapshot_ptr.is_null() {
                return Err(TestError::Bridge("Failed to create snapshot".to_string()));
            }

            let snapshot = std::slice::from_raw_parts(snapshot_ptr as *const u8, size).to_vec();

            ios_bridge_free_data(snapshot_ptr);

            Ok(snapshot)
        }
    }

    fn restore_snapshot(&self, data: &[u8]) -> Result<()> {
        if self.bridge.is_none() {
            return Ok(());
        }

        unsafe {
            ios_bridge_restore_snapshot(
                self.bridge.unwrap(),
                data.as_ptr() as *const c_void,
                data.len(),
            );
        }

        Ok(())
    }
}

unsafe extern "C" {
    fn ios_bridge_execute_action(
        bridge: *mut IOSBridge,
        action: *const c_char,
        params: *const c_char,
    ) -> *mut c_char;

    fn ios_bridge_get_current_state(bridge: *mut IOSBridge) -> *mut c_char;

    fn ios_bridge_mutate_state(
        bridge: *mut IOSBridge,
        entity: *const c_char,
        action: *const c_char,
        data: *const c_char,
    ) -> *mut c_char;

    fn ios_bridge_create_snapshot(bridge: *mut IOSBridge, size: *mut usize) -> *mut c_void;

    fn ios_bridge_restore_snapshot(bridge: *mut IOSBridge, data: *const c_void, size: usize);

    fn ios_bridge_free_string(s: *mut c_char);

    fn ios_bridge_free_data(data: *mut c_void);
}

impl Default for RustTestHarness {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl Send for RustTestHarness {}
unsafe impl Sync for RustTestHarness {}
