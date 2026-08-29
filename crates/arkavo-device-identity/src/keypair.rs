use crate::{DeviceIdentityError, Result};

const KEYPAIR_FILENAME: &str = "agent_keypair";

/// Filename for the agent's own identity keypair, kept separate from the
/// device keypair above. Note the naming trap: `KEYPAIR_FILENAME`
/// ("agent_keypair") is actually the *device* slot — existing installs
/// depend on that exact value, so it is not renamed. This constant is the
/// real per-agent identity slot, used when an agent requests its own
/// short-lived credentials distinct from its host device's identity.
const AGENT_KEYPAIR_FILENAME: &str = "agent_identity_keypair";

/// Write `bytes` to `path` so the file is never readable by anyone but its
/// owner, not even for an instant: the content is staged in a sibling file
/// created with mode 0600 and then renamed over the target. Writing in place
/// and tightening the mode afterwards leaves a world-readable window over a
/// private key, and lets a concurrent reader see a half-written file.
fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let stem = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("keypair");
    let tmp = path.with_file_name(format!(
        ".{stem}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    // A leftover from a killed run would defeat `create_new`, which is what
    // guarantees the mode is applied rather than inherited.
    let _ = std::fs::remove_file(&tmp);

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let staged = (|| -> std::io::Result<()> {
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();

    staged.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        DeviceIdentityError::Storage(format!("Failed to write file: {}", e))
    })
}

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
        super::write_private(&keypair_path(filename)?, keypair_bytes)
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

    #[cfg(any(test, feature = "test-utils"))]
    pub fn slot_path(filename: &str) -> Result<PathBuf> {
        keypair_path(filename)
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
        super::write_private(&keypair_path(filename)?, keypair_bytes)
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

    #[cfg(any(test, feature = "test-utils"))]
    pub fn slot_path(filename: &str) -> Result<PathBuf> {
        keypair_path(filename)
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
        super::write_private(&keypair_path(filename)?, keypair_bytes)
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

    #[cfg(any(test, feature = "test-utils"))]
    pub fn slot_path(filename: &str) -> Result<PathBuf> {
        keypair_path(filename)
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

/// Filesystem locations of the device and agent keypair slots, in that order.
/// Test support needs them to save and restore whatever the developer's real
/// installation held before a test overwrote the slots.
#[cfg(any(test, feature = "test-utils"))]
pub(crate) fn slot_paths() -> Result<Vec<std::path::PathBuf>> {
    Ok(vec![
        platform::slot_path(KEYPAIR_FILENAME)?,
        platform::slot_path(AGENT_KEYPAIR_FILENAME)?,
    ])
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
    use crate::test_utils::KeypairSlotGuard;
    use arkavo_test_macros::spec;
    use std::sync::Mutex;

    // Mutex to serialize tests that access the system keychain
    static KEYCHAIN_MUTEX: Mutex<()> = Mutex::new(());

    #[spec("DEVICE-007")]
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn agent_slot_is_independent_of_device_slot() {
        let _lock = KEYCHAIN_MUTEX.lock().unwrap();
        let _slots = KeypairSlotGuard::capture();

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
    }

    #[spec("DEVICE-007")]
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn agent_did_differs_from_device_did() {
        let _lock = KEYCHAIN_MUTEX.lock().unwrap();
        let _slots = KeypairSlotGuard::capture();

        let device_kp = arkavo_crypto::AgentKeypair::generate();
        store_keypair(&device_kp.to_bytes()).unwrap();
        let agent_kp = arkavo_crypto::AgentKeypair::generate();
        store_agent_keypair(&agent_kp.to_bytes()).unwrap();

        let device_did = arkavo_crypto::AgentKeypair::from_bytes(&get_keypair().unwrap().unwrap())
            .unwrap()
            .public_key()
            .to_did_key();
        let agent_did =
            arkavo_crypto::AgentKeypair::from_bytes(&get_agent_keypair().unwrap().unwrap())
                .unwrap()
                .public_key()
                .to_did_key();

        assert_ne!(
            device_did, agent_did,
            "agent DID must differ from device DID"
        );
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    fn test_keypair_storage() {
        let _lock = KEYCHAIN_MUTEX.lock().unwrap();
        let _slots = KeypairSlotGuard::capture();

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
        let _lock = KEYCHAIN_MUTEX.lock().unwrap();
        let _slots = KeypairSlotGuard::capture();

        let result = get_keypair().expect("get_keypair should not fail");
        assert!(result.is_none());
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn test_keypair_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let _lock = KEYCHAIN_MUTEX.lock().unwrap();
        let _slots = KeypairSlotGuard::capture();

        store_keypair(&[1u8, 2, 3, 4]).expect("Failed to store keypair");

        let path = &slot_paths().unwrap()[0];
        let metadata = std::fs::metadata(path).expect("Failed to get metadata");
        let permissions = metadata.permissions();
        assert_eq!(permissions.mode() & 0o777, 0o600);

        delete_keypair().expect("Failed to delete keypair");
    }

    /// Regression: the keypair used to be written with `fs::write` and only
    /// then chmod-ed to 0600, so it existed world-readable for an instant and
    /// inherited a pre-existing file's wider mode until the chmod landed. The
    /// replacement is now staged in a sibling created 0600 and renamed over the
    /// target, so a wider mode on the old file cannot survive the write and no
    /// staging file is left behind.
    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn store_keypair_never_inherits_a_world_readable_mode() {
        use std::os::unix::fs::PermissionsExt;

        let _lock = KEYCHAIN_MUTEX.lock().unwrap();
        let _slots = KeypairSlotGuard::capture();

        let path = slot_paths().unwrap()[0].clone();
        std::fs::write(&path, b"pre-existing").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        store_keypair(&[7u8; 32]).expect("Failed to store keypair");

        assert_eq!(get_keypair().unwrap().unwrap(), vec![7u8; 32]);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600,
            "a replacement must be created 0600, never inherit the old mode"
        );

        let leftovers: Vec<String> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging files left behind: {leftovers:?}"
        );

        delete_keypair().expect("Failed to delete keypair");
    }
}
