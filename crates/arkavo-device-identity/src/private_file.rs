use crate::{DeviceIdentityError, Result};
use std::path::{Path, PathBuf};

/// Write `bytes` to `path` so the file is never readable by anyone but its
/// owner, not even for an instant: the content is staged in a sibling file
/// created with mode 0600 and then renamed over the target. Writing in place
/// and tightening the mode afterwards leaves a world-readable window over a
/// private key, and lets a concurrent reader see a half-written file.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let staged = stage(path, bytes)?;
    std::fs::rename(&staged, path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        DeviceIdentityError::Storage(format!("Failed to write file: {}", e))
    })
}

/// Like [`write_private`], but only if nothing is at `path` yet. Returns
/// `false` when another process got there first, so two processes starting
/// at once settle on one file instead of the later one replacing the earlier.
pub(crate) fn create_private(path: &Path, bytes: &[u8]) -> Result<bool> {
    let staged = stage(path, bytes)?;
    // A hard link, unlike a rename, refuses to replace an existing file.
    let linked = std::fs::hard_link(&staged, path);
    let _ = std::fs::remove_file(&staged);
    match linked {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(DeviceIdentityError::Storage(format!(
            "Failed to write file: {}",
            e
        ))),
    }
}

/// Make `dir` and its missing parents, the new ones readable by their owner
/// only: the directory names which agents a user runs.
pub(crate) fn create_private_dir(dir: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(|e| {
        DeviceIdentityError::Storage(format!("Failed to create {}: {e}", dir.display()))
    })
}

fn stage(path: &Path, bytes: &[u8]) -> Result<PathBuf> {
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
        file.sync_all()
    })();

    staged.map(|()| tmp.clone()).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        DeviceIdentityError::Storage(format!("Failed to write file: {}", e))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = crate::test_storage_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn create_private_keeps_the_file_that_was_there_first() {
        let path = scratch("create_private_first");
        assert!(create_private(&path, b"first").unwrap());
        assert!(!create_private(&path, b"second").unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        let leftovers = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(leftovers, 0, "staging files must not be left behind");
    }

    #[cfg(unix)]
    #[test]
    fn create_private_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch("create_private_mode");
        assert!(create_private(&path, b"secret").unwrap());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
