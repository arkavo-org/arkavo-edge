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
