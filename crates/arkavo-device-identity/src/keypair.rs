use crate::{DeviceIdentityError, Result};

const KEYPAIR_FILENAME: &str = "agent_keypair";

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn get_keypair_path() -> Result<PathBuf> {
        let mut path = dirs::home_dir().ok_or_else(|| {
            DeviceIdentityError::Storage("Could not determine home directory".to_string())
        })?;
        path.push("Library");
        path.push("Application Support");
        path.push("arkavo");
        fs::create_dir_all(&path).map_err(|e| {
            DeviceIdentityError::Storage(format!("Failed to create directory: {}", e))
        })?;
        path.push(KEYPAIR_FILENAME);
        Ok(path)
    }

    pub fn get() -> Result<Option<Vec<u8>>> {
        let path = get_keypair_path()?;
        if !path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read file: {}", e)))?;

        Ok(Some(bytes))
    }

    pub fn store(keypair_bytes: &[u8]) -> Result<()> {
        let path = get_keypair_path()?;

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

    pub fn delete() -> Result<()> {
        let path = get_keypair_path()?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| {
                DeviceIdentityError::Storage(format!("Failed to delete file: {}", e))
            })?;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn get_keypair_path() -> Result<PathBuf> {
        let mut path = dirs::data_local_dir().ok_or_else(|| {
            DeviceIdentityError::Storage("Could not determine local data directory".to_string())
        })?;
        path.push("arkavo");
        fs::create_dir_all(&path).map_err(|e| {
            DeviceIdentityError::Storage(format!("Failed to create directory: {}", e))
        })?;
        path.push(KEYPAIR_FILENAME);
        Ok(path)
    }

    pub fn get() -> Result<Option<Vec<u8>>> {
        let path = get_keypair_path()?;
        if !path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read file: {}", e)))?;

        Ok(Some(bytes))
    }

    pub fn store(keypair_bytes: &[u8]) -> Result<()> {
        let path = get_keypair_path()?;

        fs::write(&path, keypair_bytes)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to write file: {}", e)))?;

        use std::os::unix::fs::PermissionsExt;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path, permissions).map_err(|e| {
            DeviceIdentityError::Storage(format!("Failed to set permissions: {}", e))
        })?;

        Ok(())
    }

    pub fn delete() -> Result<()> {
        let path = get_keypair_path()?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| {
                DeviceIdentityError::Storage(format!("Failed to delete file: {}", e))
            })?;
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn get_keypair_path() -> Result<PathBuf> {
        let mut path = dirs::data_local_dir().ok_or_else(|| {
            DeviceIdentityError::Storage("Could not determine local data directory".to_string())
        })?;
        path.push("arkavo");
        fs::create_dir_all(&path).map_err(|e| {
            DeviceIdentityError::Storage(format!("Failed to create directory: {}", e))
        })?;
        path.push(KEYPAIR_FILENAME);
        Ok(path)
    }

    pub fn get() -> Result<Option<Vec<u8>>> {
        let path = get_keypair_path()?;
        if !path.exists() {
            return Ok(None);
        }

        let bytes = fs::read(&path)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to read file: {}", e)))?;

        Ok(Some(bytes))
    }

    pub fn store(keypair_bytes: &[u8]) -> Result<()> {
        let path = get_keypair_path()?;

        fs::write(&path, keypair_bytes)
            .map_err(|e| DeviceIdentityError::Storage(format!("Failed to write file: {}", e)))?;

        Ok(())
    }

    pub fn delete() -> Result<()> {
        let path = get_keypair_path()?;
        if path.exists() {
            fs::remove_file(&path).map_err(|e| {
                DeviceIdentityError::Storage(format!("Failed to delete file: {}", e))
            })?;
        }
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn test_keypair_storage() {
        let _ = delete_keypair();

        let test_data = vec![1u8, 2, 3, 4, 5];
        store_keypair(&test_data).expect("Failed to store keypair");

        let retrieved = get_keypair()
            .expect("Failed to get keypair")
            .expect("Keypair not found");

        assert_eq!(test_data, retrieved);

        delete_keypair().expect("Failed to delete keypair");
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn test_keypair_nonexistent() {
        let _ = delete_keypair();

        let result = get_keypair().expect("get_keypair should not fail");
        assert!(result.is_none());
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn test_keypair_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let _ = delete_keypair();

        let test_data = vec![1u8, 2, 3, 4];
        store_keypair(&test_data).expect("Failed to store keypair");

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
