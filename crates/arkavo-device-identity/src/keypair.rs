use crate::{DeviceIdentityError, Result};

const KEYPAIR_FILENAME: &str = "agent_keypair";

/// Filename for the agent's own identity keypair, kept separate from the
/// device keypair above. Note the naming trap: `KEYPAIR_FILENAME`
/// ("agent_keypair") is actually the *device* slot — existing installs
/// depend on that exact value, so it is not renamed. This constant is the
/// real per-agent identity slot, used when an agent requests its own
/// short-lived credentials distinct from its host device's identity.
const AGENT_KEYPAIR_FILENAME: &str = "agent_identity_keypair";

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn keypair_path(filename: &str) -> Result<PathBuf> {
        let mut path = dirs::home_dir().ok_or_else(|| {
            DeviceIdentityError::Storage("Could not determine home directory".to_string())
        })?;
        path.push("Library");
        path.push("Application Support");
        path.push("arkavo");
        fs::create_dir_all(&path).map_err(|e| {
            DeviceIdentityError::Storage(format!("Failed to create directory: {}", e))
        })?;
        path.push(filename);
        Ok(path)
    }

    fn read(filename: &str) -> Result<Option<Vec<u8>>> {
        let path = keypair_path(filename)?;
        if !path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read file: {}", e)))?;

        Ok(Some(bytes))
    }

    fn write(filename: &str, keypair_bytes: &[u8]) -> Result<()> {
        let path = keypair_path(filename)?;

        fs::write(&path, keypair_bytes)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to write file: {}", e)))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&path, permissions).map_err(|e| {
                DeviceIdentityError::Storage(format!("Failed to set permissions: {}", e))
            })?;
        }

        Ok(())
    }

    fn remove(filename: &str) -> Result<()> {
        let path = keypair_path(filename)?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| {
                DeviceIdentityError::Storage(format!("Failed to delete file: {}", e))
            })?;
        }
        Ok(())
    }

    pub fn get() -> Result<Option<Vec<u8>>> {
        read(KEYPAIR_FILENAME)
    }

    pub fn store(keypair_bytes: &[u8]) -> Result<()> {
        write(KEYPAIR_FILENAME, keypair_bytes)
    }

    pub fn delete() -> Result<()> {
        remove(KEYPAIR_FILENAME)
    }

    pub fn get_agent() -> Result<Option<Vec<u8>>> {
        read(AGENT_KEYPAIR_FILENAME)
    }

    pub fn store_agent(keypair_bytes: &[u8]) -> Result<()> {
        write(AGENT_KEYPAIR_FILENAME, keypair_bytes)
    }

    pub fn delete_agent() -> Result<()> {
        remove(AGENT_KEYPAIR_FILENAME)
    }

    pub fn created_at() -> Result<Option<std::time::SystemTime>> {
        let path = keypair_path(KEYPAIR_FILENAME)?;
        if !path.exists() {
            return Ok(None);
        }
        let meta = fs::metadata(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to stat file: {}", e)))?;
        // Prefer mtime — present on every platform — over creation time
        // (`created()` returns Err on some Linux filesystems). The keypair
        // file is written once and never modified, so mtime ≈ birth time.
        let ts = meta
            .modified()
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read mtime: {}", e)))?;
        Ok(Some(ts))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn keypair_path(filename: &str) -> Result<PathBuf> {
        let mut path = dirs::data_local_dir().ok_or_else(|| {
            DeviceIdentityError::Storage("Could not determine local data directory".to_string())
        })?;
        path.push("arkavo");
        fs::create_dir_all(&path).map_err(|e| {
            DeviceIdentityError::Storage(format!("Failed to create directory: {}", e))
        })?;
        path.push(filename);
        Ok(path)
    }

    fn read(filename: &str) -> Result<Option<Vec<u8>>> {
        let path = keypair_path(filename)?;
        if !path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read file: {}", e)))?;

        Ok(Some(bytes))
    }

    fn write(filename: &str, keypair_bytes: &[u8]) -> Result<()> {
        let path = keypair_path(filename)?;

        fs::write(&path, keypair_bytes)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to write file: {}", e)))?;

        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path, permissions).map_err(|e| {
            DeviceIdentityError::Storage(format!("Failed to set permissions: {}", e))
        })?;

        Ok(())
    }

    fn remove(filename: &str) -> Result<()> {
        let path = keypair_path(filename)?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| {
                DeviceIdentityError::Storage(format!("Failed to delete file: {}", e))
            })?;
        }
        Ok(())
    }

    pub fn get() -> Result<Option<Vec<u8>>> {
        read(KEYPAIR_FILENAME)
    }

    pub fn store(keypair_bytes: &[u8]) -> Result<()> {
        write(KEYPAIR_FILENAME, keypair_bytes)
    }

    pub fn delete() -> Result<()> {
        remove(KEYPAIR_FILENAME)
    }

    pub fn get_agent() -> Result<Option<Vec<u8>>> {
        read(AGENT_KEYPAIR_FILENAME)
    }

    pub fn store_agent(keypair_bytes: &[u8]) -> Result<()> {
        write(AGENT_KEYPAIR_FILENAME, keypair_bytes)
    }

    pub fn delete_agent() -> Result<()> {
        remove(AGENT_KEYPAIR_FILENAME)
    }

    pub fn created_at() -> Result<Option<std::time::SystemTime>> {
        let path = keypair_path(KEYPAIR_FILENAME)?;
        if !path.exists() {
            return Ok(None);
        }
        let meta = fs::metadata(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to stat file: {}", e)))?;
        // Prefer mtime — present on every platform — over creation time
        // (`created()` returns Err on some Linux filesystems). The keypair
        // file is written once and never modified, so mtime ≈ birth time.
        let ts = meta
            .modified()
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read mtime: {}", e)))?;
        Ok(Some(ts))
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn keypair_path(filename: &str) -> Result<PathBuf> {
        let mut path = dirs::data_local_dir().ok_or_else(|| {
            DeviceIdentityError::Storage("Could not determine local data directory".to_string())
        })?;
        path.push("arkavo");
        fs::create_dir_all(&path).map_err(|e| {
            DeviceIdentityError::Storage(format!("Failed to create directory: {}", e))
        })?;
        path.push(filename);
        Ok(path)
    }

    fn read(filename: &str) -> Result<Option<Vec<u8>>> {
        let path = keypair_path(filename)?;
        if !path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read file: {}", e)))?;

        Ok(Some(bytes))
    }

    fn write(filename: &str, keypair_bytes: &[u8]) -> Result<()> {
        let path = keypair_path(filename)?;

        fs::write(&path, keypair_bytes)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to write file: {}", e)))?;

        Ok(())
    }

    fn remove(filename: &str) -> Result<()> {
        let path = keypair_path(filename)?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| {
                DeviceIdentityError::Storage(format!("Failed to delete file: {}", e))
            })?;
        }
        Ok(())
    }

    pub fn get() -> Result<Option<Vec<u8>>> {
        read(KEYPAIR_FILENAME)
    }

    pub fn store(keypair_bytes: &[u8]) -> Result<()> {
        write(KEYPAIR_FILENAME, keypair_bytes)
    }

    pub fn delete() -> Result<()> {
        remove(KEYPAIR_FILENAME)
    }

    pub fn get_agent() -> Result<Option<Vec<u8>>> {
        read(AGENT_KEYPAIR_FILENAME)
    }

    pub fn store_agent(keypair_bytes: &[u8]) -> Result<()> {
        write(AGENT_KEYPAIR_FILENAME, keypair_bytes)
    }

    pub fn delete_agent() -> Result<()> {
        remove(AGENT_KEYPAIR_FILENAME)
    }

    pub fn created_at() -> Result<Option<std::time::SystemTime>> {
        let path = keypair_path(KEYPAIR_FILENAME)?;
        if !path.exists() {
            return Ok(None);
        }
        let meta = fs::metadata(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to stat file: {}", e)))?;
        // Prefer mtime — present on every platform — over creation time
        // (`created()` returns Err on some Linux filesystems). The keypair
        // file is written once and never modified, so mtime ≈ birth time.
        let ts = meta
            .modified()
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read mtime: {}", e)))?;
        Ok(Some(ts))
    }
}

pub fn get_keypair() -> Result<Option<Vec<u8>>> {
    platform::get()
}

pub fn store_keypair(keypair_bytes: &[u8]) -> Result<()> {
    platform::store(keypair_bytes)
}

pub fn delete_keypair() -> Result<()> {
    platform::delete()
}

/// Retrieve the agent's own identity keypair. This is stored in a slot
/// separate from `get_keypair`'s device keypair, so an agent process can
/// hold a distinct identity from the device it runs on.
pub fn get_agent_keypair() -> Result<Option<Vec<u8>>> {
    platform::get_agent()
}

/// Persist the agent's own identity keypair, independent of the device slot.
pub fn store_agent_keypair(keypair_bytes: &[u8]) -> Result<()> {
    platform::store_agent(keypair_bytes)
}

/// Remove the agent's own identity keypair. Leaves the device keypair intact.
pub fn delete_agent_keypair() -> Result<()> {
    platform::delete_agent()
}

/// File mtime of the persistent keypair, used as the agent's "birth time"
/// for the MCP-T tenure dimension. Returns `Ok(None)` if the keypair has
/// not been created yet.
pub fn created_at() -> Result<Option<std::time::SystemTime>> {
    platform::created_at()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use std::sync::Mutex;

    // Mutex to serialize tests that access the system keychain
    static KEYCHAIN_MUTEX: Mutex<()> = Mutex::new(());

    #[spec("DEVICE-007")]
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn agent_slot_is_independent_of_device_slot() {
        let _g = KEYCHAIN_MUTEX.lock().unwrap();
        let _ = delete_keypair();
        let _ = delete_agent_keypair();
        store_keypair(&[1u8; 64]).unwrap();
        assert_eq!(
            get_agent_keypair().unwrap(),
            None,
            "agent slot must start empty"
        );
        store_agent_keypair(&[2u8; 64]).unwrap();
        assert_eq!(get_keypair().unwrap().unwrap(), vec![1u8; 64]);
        assert_eq!(get_agent_keypair().unwrap().unwrap(), vec![2u8; 64]);
        delete_agent_keypair().unwrap();
        assert_eq!(get_agent_keypair().unwrap(), None);
        let _ = delete_keypair();
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn test_keypair_storage() {
        let _guard = KEYCHAIN_MUTEX.lock().unwrap();
        let _ = delete_keypair();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let test_data = vec![1u8, 2, 3, 4, 5];
        store_keypair(&test_data).expect("Failed to store keypair");
        std::thread::sleep(std::time::Duration::from_millis(50));

        let retrieved = get_keypair()
            .expect("Failed to get keypair")
            .expect("Keypair not found");

        assert_eq!(test_data, retrieved);

        delete_keypair().expect("Failed to delete keypair");
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn test_keypair_nonexistent() {
        let _guard = KEYCHAIN_MUTEX.lock().unwrap();
        let _ = delete_keypair();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let result = get_keypair().expect("get_keypair should not fail");
        assert!(result.is_none());
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn test_keypair_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let _guard = KEYCHAIN_MUTEX.lock().unwrap();
        let _ = delete_keypair();
        std::thread::sleep(std::time::Duration::from_millis(50));

        let test_data = vec![1u8, 2, 3, 4];
        store_keypair(&test_data).expect("Failed to store keypair");
        std::thread::sleep(std::time::Duration::from_millis(50));

        let path = if cfg!(target_os = "macos") {
            let mut p = dirs::home_dir().unwrap();
            p.push("Library/Application Support/arkavo/agent_keypair");
            p
        } else {
            let mut p = dirs::data_local_dir().unwrap();
            p.push("arkavo/agent_keypair");
            p
        };

        let metadata = std::fs::metadata(&path).expect("Failed to get metadata");
        let permissions = metadata.permissions();
        assert_eq!(permissions.mode() & 0o777, 0o600);

        delete_keypair().expect("Failed to delete keypair");
    }
}
