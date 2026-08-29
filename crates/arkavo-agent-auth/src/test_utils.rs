//! Test-only protection for the on-disk agent token.
//!
//! The token file is the developer's real credential, minted against the agent
//! DID a human approved in authnz-rs. A test that stores or deletes it must put
//! back exactly what it found — including putting back "nothing" — or running
//! the suite drops a live credential on the floor.

use std::path::PathBuf;

/// RAII guard that empties the agent token file for the duration of a test and
/// restores its previous contents when it drops.
///
/// The restore lives in `Drop`, not at the end of the test body, because a
/// failing `assert!` unwinds past trailing cleanup code but not past a
/// destructor. Bind it *after* the test's serialising mutex guard so the file
/// is back before another test can take the lock.
pub struct TokenFileGuard {
    path: PathBuf,
    saved: Option<Vec<u8>>,
}

impl TokenFileGuard {
    /// Capture the token file and clear it, so the test starts from a
    /// known-empty state regardless of what the developer's machine holds.
    pub fn capture() -> Self {
        let path = crate::storage::get_token_path().expect("locate the agent token file");
        let saved = std::fs::read(&path).ok();
        if saved.is_some() {
            let _ = std::fs::remove_file(&path);
        }
        Self { path, saved }
    }
}

impl Drop for TokenFileGuard {
    fn drop(&mut self) {
        let restored = match &self.saved {
            Some(bytes) => restore(&self.path, bytes),
            None => match std::fs::remove_file(&self.path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            },
        };
        // Panicking here would abort the process during unwind and hide
        // whichever assertion actually failed.
        if let Err(e) = restored {
            eprintln!("failed to restore agent token {}: {e}", self.path.display());
        }
    }
}

/// Write the captured bytes back with owner-only permissions. `Drop` cannot
/// await, so this is the synchronous counterpart to `storage::write_private`
/// rather than a call into it.
fn restore(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    options.open(path)?.write_all(bytes)
}
