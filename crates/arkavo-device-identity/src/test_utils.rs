//! Test-only protection for the on-disk keypair slots.
//!
//! The slots are real files under the developer's home directory, and the
//! agent slot holds the identity a human approved in authnz-rs. A test that
//! writes or deletes them must therefore put back exactly what it found —
//! including putting back "nothing" — or running the suite silently orphans
//! that approval and forces the human to re-`--trust` the agent.

use std::path::PathBuf;
use std::time::SystemTime;

/// A keypair slot's previous contents, captured so they can be restored:
/// the file's bytes and its mtime, if the slot existed at all.
type SavedSlotContent = Option<(Vec<u8>, Option<SystemTime>)>;

/// RAII guard that empties both keypair slots for the duration of a test and
/// restores their previous contents when it drops.
///
/// The restore lives in `Drop`, not at the end of the test body, because a
/// failing `assert!` unwinds past trailing cleanup code but not past a
/// destructor. Bind it *after* the test's serialising mutex guard so the files
/// are back before another test can take the lock.
pub struct KeypairSlotGuard {
    saved: Vec<(PathBuf, SavedSlotContent)>,
}

impl KeypairSlotGuard {
    /// Capture both keypair slots and clear them, so the test starts from a
    /// known-empty state regardless of what the developer's machine holds.
    pub fn capture() -> Self {
        let paths = crate::keypair::slot_paths().expect("locate the keypair slots");
        let saved = paths
            .into_iter()
            .map(|path| {
                let content = std::fs::read(&path).ok().map(|bytes| {
                    // `keypair::created_at` reads mtime as the agent's birth
                    // time, so the restore has to put the timestamp back too.
                    let mtime = std::fs::metadata(&path)
                        .ok()
                        .and_then(|m| m.modified().ok());
                    (bytes, mtime)
                });
                if content.is_some() {
                    let _ = std::fs::remove_file(&path);
                }
                (path, content)
            })
            .collect();
        Self { saved }
    }
}

impl Drop for KeypairSlotGuard {
    fn drop(&mut self) {
        for (path, content) in &self.saved {
            let restored = match content {
                Some((bytes, mtime)) => restore(path, bytes, *mtime),
                None => match std::fs::remove_file(path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    other => other,
                },
            };
            // Panicking here would abort the process during unwind and hide
            // whichever assertion actually failed.
            if let Err(e) = restored {
                eprintln!("failed to restore keypair slot {}: {e}", path.display());
            }
        }
    }
}

/// Write the captured bytes back with owner-only permissions. This mirrors
/// `keypair`'s own private write rather than calling it, so the guard keeps
/// working as a safety net even if that production path regresses.
fn restore(path: &std::path::Path, bytes: &[u8], mtime: Option<SystemTime>) -> std::io::Result<()> {
    use std::io::Write;

    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    if let Some(mtime) = mtime {
        file.set_times(std::fs::FileTimes::new().set_modified(mtime))?;
    }
    Ok(())
}

/// RAII guard that empties the `device_id` slot for the duration of a test
/// and restores its previous contents when it drops.
///
/// Same contract as [`KeypairSlotGuard`]: the restore lives in `Drop` because
/// a failing `assert!` unwinds past trailing cleanup. Bind it *after* the
/// test's serialising mutex so the slot is back before another test can take
/// the lock. File bytes are preferred on restore because they are the
/// file-backed platforms' source of truth; the parsed id is kept so a
/// keyring-only platform can put the value back too.
pub struct DeviceIdFileGuard {
    path: PathBuf,
    saved_file: Option<Vec<u8>>,
    saved_id: Option<crate::DeviceId>,
}

impl DeviceIdFileGuard {
    /// Capture the device_id slot and clear it, so the test starts from a
    /// known-empty state regardless of what the developer's machine holds.
    pub fn capture() -> Self {
        let path = device_id_file_path();
        let saved_file = std::fs::read(&path).ok();
        let saved_id = crate::storage::get().ok().flatten();
        if saved_file.is_some() {
            let _ = std::fs::remove_file(&path);
        }
        let _ = crate::storage::delete();
        Self {
            path,
            saved_file,
            saved_id,
        }
    }
}

impl Drop for DeviceIdFileGuard {
    fn drop(&mut self) {
        match (&self.saved_file, self.saved_id) {
            (Some(bytes), _) => {
                if let Some(parent) = self.path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = restore(&self.path, bytes, None) {
                    eprintln!(
                        "failed to restore device_id file {}: {e}",
                        self.path.display()
                    );
                }
            }
            (None, Some(id)) => {
                if let Err(e) = crate::storage::store(id) {
                    eprintln!("failed to restore device_id into platform storage: {e}");
                }
            }
            (None, None) => {
                if let Err(e) = match std::fs::remove_file(&self.path) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    other => other,
                } {
                    eprintln!(
                        "failed to clear device_id file {}: {e}",
                        self.path.display()
                    );
                }
                let _ = crate::storage::delete();
            }
        }
    }
}

/// Path of the on-disk `device_id` file used by the file-backed platforms.
/// Tests that write malformed bytes must use this same path so the guard
/// restores the developer's file rather than a sibling the suite never
/// touches.
pub fn device_id_file_path() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let mut path = dirs::home_dir().expect("home directory for device_id file");
        path.push("Library/Application Support/arkavo/device_id");
        path
    }
    #[cfg(target_os = "linux")]
    {
        let mut path = dirs::data_local_dir().expect("local data directory for device_id file");
        path.push("arkavo/device_id");
        path
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        std::env::temp_dir().join("arkavo-test-device_id")
    }
}
