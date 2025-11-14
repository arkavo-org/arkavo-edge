use crate::{DeviceId, DeviceIdentityError, Result};

const DEVICE_ID_KEY: &str = "com.arkavo.edge.device_id";

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn use_file_storage() -> bool {
        true
    }

    fn get_storage_path() -> Result<PathBuf> {
        let mut path = dirs::home_dir().ok_or_else(|| {
            DeviceIdentityError::Storage("Could not determine home directory".to_string())
        })?;
        path.push("Library");
        path.push("Application Support");
        path.push("arkavo");
        fs::create_dir_all(&path).map_err(|e| {
            DeviceIdentityError::Storage(format!("Failed to create directory: {}", e))
        })?;
        path.push("device_id");
        Ok(path)
    }

    fn get_file_based() -> Result<Option<DeviceId>> {
        let path = get_storage_path()?;
        if !path.exists() {
            return Ok(None);
        }

        let hex_string = fs::read_to_string(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read file: {}", e)))?;

        let bytes = hex::decode(hex_string.trim())
            .map_err(|e| DeviceIdentityError::InvalidFormat(format!("Invalid hex: {}", e)))?;

        if bytes.len() != 16 {
            return Err(DeviceIdentityError::InvalidFormat(format!(
                "Expected 16 bytes, got {}",
                bytes.len()
            )));
        }

        let mut array = [0u8; 16];
        array.copy_from_slice(&bytes);
        Ok(Some(DeviceId::from_bytes(array)))
    }

    fn store_file_based(device_id: DeviceId) -> Result<()> {
        let path = get_storage_path()?;
        let hex_string = hex::encode(device_id.as_bytes());
        fs::write(&path, hex_string)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to write file: {}", e)))
    }

    fn delete_file_based() -> Result<()> {
        let path = get_storage_path()?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| {
                DeviceIdentityError::Storage(format!("Failed to delete file: {}", e))
            })?;
        }
        Ok(())
    }

    fn get_keychain() -> Result<Option<DeviceId>> {
        use security_framework::passwords::get_generic_password;

        match get_generic_password("arkavo-edge", DEVICE_ID_KEY) {
            Ok(bytes) => {
                if bytes.len() != 16 {
                    return Err(DeviceIdentityError::InvalidFormat(format!(
                        "Expected 16 bytes, got {}",
                        bytes.len()
                    )));
                }
                let mut array = [0u8; 16];
                array.copy_from_slice(&bytes);
                Ok(Some(DeviceId::from_bytes(array)))
            }
            Err(err) => {
                let err_string = err.to_string();
                let err_lower = err_string.to_lowercase();
                if err_lower.contains("not found") || err_lower.contains("could not be found") {
                    Ok(None)
                } else {
                    Err(DeviceIdentityError::Storage(err_string))
                }
            }
        }
    }

    fn store_keychain(device_id: DeviceId) -> Result<()> {
        use security_framework::passwords::set_generic_password;

        set_generic_password("arkavo-edge", DEVICE_ID_KEY, device_id.as_bytes())
            .map_err(|e| DeviceIdentityError::Storage(e.to_string()))
    }

    fn delete_keychain() -> Result<()> {
        use security_framework::passwords::delete_generic_password;

        match delete_generic_password("arkavo-edge", DEVICE_ID_KEY) {
            Ok(()) => Ok(()),
            Err(err) if err.to_string().contains("not found") => Ok(()),
            Err(err) => Err(DeviceIdentityError::Storage(err.to_string())),
        }
    }

    pub fn get() -> Result<Option<DeviceId>> {
        if use_file_storage() {
            get_file_based()
        } else {
            get_keychain()
        }
    }

    pub fn store(device_id: DeviceId) -> Result<()> {
        if use_file_storage() {
            store_file_based(device_id)
        } else {
            store_keychain(device_id)
        }
    }

    pub fn delete() -> Result<()> {
        if use_file_storage() {
            delete_file_based()
        } else {
            delete_keychain()
        }
    }
}

#[cfg(all(target_os = "linux", feature = "linux-keyring"))]
mod platform {
    use super::*;
    use keyring::Entry;

    fn get_entry() -> Result<Entry> {
        Entry::new("arkavo-edge", DEVICE_ID_KEY)
            .map_err(|e| DeviceIdentityError::Storage(e.to_string()))
    }

    pub fn get() -> Result<Option<DeviceId>> {
        let entry = get_entry()?;
        match entry.get_password() {
            Ok(password) => {
                let bytes = hex::decode(&password).map_err(|e| {
                    DeviceIdentityError::InvalidFormat(format!("Invalid hex: {}", e))
                })?;
                if bytes.len() != 16 {
                    return Err(DeviceIdentityError::InvalidFormat(format!(
                        "Expected 16 bytes, got {}",
                        bytes.len()
                    )));
                }
                let mut array = [0u8; 16];
                array.copy_from_slice(&bytes);
                Ok(Some(DeviceId::from_bytes(array)))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(DeviceIdentityError::Storage(err.to_string())),
        }
    }

    pub fn store(device_id: DeviceId) -> Result<()> {
        let entry = get_entry()?;
        let hex_string = hex::encode(device_id.as_bytes());
        entry
            .set_password(&hex_string)
            .map_err(|e| DeviceIdentityError::Storage(e.to_string()))
    }

    pub fn delete() -> Result<()> {
        let entry = get_entry()?;
        entry
            .delete_credential()
            .map_err(|e| DeviceIdentityError::Storage(e.to_string()))
    }
}

#[cfg(target_os = "linux")]
use hex;

#[cfg(all(target_os = "linux", not(feature = "linux-keyring")))]
mod platform {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn get_storage_path() -> Result<PathBuf> {
        let mut path = dirs::data_local_dir().ok_or_else(|| {
            DeviceIdentityError::Storage("Could not determine local data directory".to_string())
        })?;
        path.push("arkavo");
        fs::create_dir_all(&path).map_err(|e| {
            DeviceIdentityError::Storage(format!("Failed to create directory: {}", e))
        })?;
        path.push("device_id");
        Ok(path)
    }

    pub fn get() -> Result<Option<DeviceId>> {
        let path = get_storage_path()?;
        if !path.exists() {
            return Ok(None);
        }

        let hex_string = fs::read_to_string(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read file: {}", e)))?;

        let bytes = hex::decode(hex_string.trim())
            .map_err(|e| DeviceIdentityError::InvalidFormat(format!("Invalid hex: {}", e)))?;

        if bytes.len() != 16 {
            return Err(DeviceIdentityError::InvalidFormat(format!(
                "Expected 16 bytes, got {}",
                bytes.len()
            )));
        }

        let mut array = [0u8; 16];
        array.copy_from_slice(&bytes);
        Ok(Some(DeviceId::from_bytes(array)))
    }

    pub fn store(device_id: DeviceId) -> Result<()> {
        let path = get_storage_path()?;
        let hex_string = hex::encode(device_id.as_bytes());
        fs::write(&path, hex_string)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to write file: {}", e)))
    }

    pub fn delete() -> Result<()> {
        let path = get_storage_path()?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| {
                DeviceIdentityError::Storage(format!("Failed to delete file: {}", e))
            })?;
        }
        Ok(())
    }
}

#[cfg(all(target_os = "linux", not(feature = "linux-keyring")))]
use dirs;

#[cfg(all(target_os = "windows", feature = "windows-credential"))]
mod platform {
    use super::*;

    pub fn get() -> Result<Option<DeviceId>> {
        Err(DeviceIdentityError::UnsupportedPlatform(
            "Windows credential storage not yet implemented".to_string(),
        ))
    }

    pub fn store(_device_id: DeviceId) -> Result<()> {
        Err(DeviceIdentityError::UnsupportedPlatform(
            "Windows credential storage not yet implemented".to_string(),
        ))
    }

    pub fn delete() -> Result<()> {
        Err(DeviceIdentityError::UnsupportedPlatform(
            "Windows credential storage not yet implemented".to_string(),
        ))
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "linux",
    all(target_os = "windows", feature = "windows-credential")
)))]
mod platform {
    use super::*;

    pub fn get() -> Result<Option<DeviceId>> {
        Err(DeviceIdentityError::UnsupportedPlatform(
            "Current platform not supported".to_string(),
        ))
    }

    pub fn store(_device_id: DeviceId) -> Result<()> {
        Err(DeviceIdentityError::UnsupportedPlatform(
            "Current platform not supported".to_string(),
        ))
    }

    pub fn delete() -> Result<()> {
        Err(DeviceIdentityError::UnsupportedPlatform(
            "Current platform not supported".to_string(),
        ))
    }
}

pub fn get() -> Result<Option<DeviceId>> {
    platform::get()
}

pub fn store(device_id: DeviceId) -> Result<()> {
    platform::store(device_id)
}

pub fn get_or_create() -> Result<DeviceId> {
    if let Some(device_id) = get()? {
        Ok(device_id)
    } else {
        let device_id = DeviceId::new();
        store(device_id)?;
        Ok(device_id)
    }
}

#[allow(dead_code)]
pub fn delete() -> Result<()> {
    platform::delete()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn test_get_or_create() {
        let _ = delete();

        let id1 = get_or_create().expect("Failed to create device ID");
        let id2 = get_or_create().expect("Failed to retrieve device ID");

        assert_eq!(id1, id2);

        let _ = delete();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn test_store_and_retrieve() {
        let _ = delete();

        let original = DeviceId::new();
        store(original).expect("Failed to store device ID");

        let retrieved = get()
            .expect("Failed to get device ID")
            .expect("Device ID not found");

        assert_eq!(original, retrieved);

        let _ = delete();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn test_malformed_device_id() {
        use std::fs;

        let _ = delete();

        #[cfg(target_os = "macos")]
        let storage_path = {
            let mut path = dirs::home_dir().unwrap();
            path.push("Library/Application Support/arkavo/device_id");
            path
        };

        #[cfg(target_os = "linux")]
        let storage_path = {
            let mut path = dirs::data_local_dir().unwrap();
            path.push("arkavo/device_id");
            path
        };

        fs::write(&storage_path, "invalid_hex_data").expect("Failed to write malformed data");

        let result = get();
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DeviceIdentityError::InvalidFormat(_)
        ));

        fs::remove_file(&storage_path).ok();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn test_wrong_size_device_id() {
        use std::fs;

        let _ = delete();

        #[cfg(target_os = "macos")]
        let storage_path = {
            let mut path = dirs::home_dir().unwrap();
            path.push("Library/Application Support/arkavo/device_id");
            path
        };

        #[cfg(target_os = "linux")]
        let storage_path = {
            let mut path = dirs::data_local_dir().unwrap();
            path.push("arkavo/device_id");
            path
        };

        let wrong_size_data = hex::encode(&[0u8; 8]);
        fs::write(&storage_path, wrong_size_data).expect("Failed to write wrong size data");

        let result = get();
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, DeviceIdentityError::InvalidFormat(_)));
        assert!(err.to_string().contains("Expected 16 bytes"));

        fs::remove_file(&storage_path).ok();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn test_delete_nonexistent() {
        let _ = delete();

        let result = delete();
        assert!(result.is_ok());
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn test_get_nonexistent() {
        let _ = delete();

        let result = get().expect("get() should not fail");
        assert!(result.is_none());

        let _ = delete();
    }
}
