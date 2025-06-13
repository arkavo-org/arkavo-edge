use crate::{idb_error_t, IdbError, Result, IDB_SUCCESS};
use crate::{
    idb_free_screenshot_shm, idb_shm_attach, idb_shm_calculate_checksum, idb_shm_create,
    idb_shm_destroy, idb_shm_detach, idb_shm_get_key, idb_shm_handle_t, idb_shm_screenshot_t,
    idb_shm_validate_screenshot, idb_take_screenshot_shm,
};
use std::ffi::CStr;
use std::ptr;
use std::slice;

pub const IDB_SHM_MAGIC_HEADER: u64 = 0x49444253484D0001;
pub const IDB_SHM_MAX_SIZE: usize = 128 * 1024 * 1024; // 128MB
pub const IDB_SHM_MIN_SIZE: usize = 1024; // 1KB

pub struct SharedMemoryHandle {
    handle: idb_shm_handle_t,
    address: *mut std::ffi::c_void,
    size: usize,
}

impl SharedMemoryHandle {
    pub fn create(size: usize) -> Result<Self> {
        if size < IDB_SHM_MIN_SIZE || size > IDB_SHM_MAX_SIZE {
            return Err(IdbError::InvalidParameter);
        }

        let mut handle: idb_shm_handle_t = ptr::null_mut();
        unsafe {
            let err = idb_shm_create(size, &mut handle);
            if err != IDB_SUCCESS {
                return Err(err.into());
            }
        }

        let mut address: *mut std::ffi::c_void = ptr::null_mut();
        unsafe {
            let err = idb_shm_attach(handle, &mut address);
            if err != IDB_SUCCESS {
                idb_shm_destroy(handle);
                return Err(err.into());
            }
        }

        Ok(Self {
            handle,
            address,
            size,
        })
    }

    pub fn key(&self) -> String {
        unsafe {
            let key_ptr = idb_shm_get_key(self.handle);
            if key_ptr.is_null() {
                String::new()
            } else {
                CStr::from_ptr(key_ptr)
                    .to_str()
                    .unwrap_or("")
                    .to_string()
            }
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.address as *const u8, self.size) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.address as *mut u8, self.size) }
    }
}

impl Drop for SharedMemoryHandle {
    fn drop(&mut self) {
        if !self.address.is_null() {
            unsafe {
                idb_shm_detach(self.address);
            }
        }
        if !self.handle.is_null() {
            unsafe {
                idb_shm_destroy(self.handle);
            }
        }
    }
}

pub struct SharedMemoryScreenshot {
    screenshot: idb_shm_screenshot_t,
}

impl SharedMemoryScreenshot {
    pub fn capture() -> Result<Self> {
        let mut screenshot = idb_shm_screenshot_t {
            magic: 0,
            handle: ptr::null_mut(),
            base_address: ptr::null_mut(),
            size: 0,
            width: 0,
            height: 0,
            bytes_per_row: 0,
            format: [0; 16],
            checksum: 0,
        };

        unsafe {
            let err = idb_take_screenshot_shm(&mut screenshot);
            if err != IDB_SUCCESS {
                return Err(err.into());
            }

            // Validate the screenshot
            let err = idb_shm_validate_screenshot(&screenshot);
            if err != IDB_SUCCESS {
                idb_free_screenshot_shm(&mut screenshot);
                return Err(err.into());
            }
        }

        Ok(Self { screenshot })
    }

    pub fn width(&self) -> u32 {
        self.screenshot.width
    }

    pub fn height(&self) -> u32 {
        self.screenshot.height
    }

    pub fn bytes_per_row(&self) -> u32 {
        self.screenshot.bytes_per_row
    }

    pub fn format(&self) -> String {
        let format_bytes = &self.screenshot.format;
        let null_pos = format_bytes.iter().position(|&c| c == 0).unwrap_or(16);
        std::str::from_utf8(&format_bytes[..null_pos])
            .unwrap_or("")
            .to_string()
    }

    pub fn data(&self) -> &[u8] {
        if self.screenshot.base_address.is_null() {
            &[]
        } else {
            unsafe {
                slice::from_raw_parts(
                    self.screenshot.base_address as *const u8,
                    self.screenshot.size,
                )
            }
        }
    }

    pub fn verify_checksum(&self) -> bool {
        unsafe {
            let calculated = idb_shm_calculate_checksum(&self.screenshot);
            calculated == self.screenshot.checksum
        }
    }

    pub fn is_valid(&self) -> bool {
        self.screenshot.magic == IDB_SHM_MAGIC_HEADER && self.verify_checksum()
    }
}

impl Drop for SharedMemoryScreenshot {
    fn drop(&mut self) {
        unsafe {
            idb_free_screenshot_shm(&mut self.screenshot);
        }
    }
}

// Zero-copy screenshot streaming
pub struct ScreenshotStream {
    callback: Box<dyn Fn(&SharedMemoryScreenshot)>,
}

impl ScreenshotStream {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(&SharedMemoryScreenshot) + 'static,
    {
        Self {
            callback: Box::new(callback),
        }
    }

    // Note: The actual streaming implementation would require more complex
    // callback handling and potentially unsafe code to interface with C callbacks
}