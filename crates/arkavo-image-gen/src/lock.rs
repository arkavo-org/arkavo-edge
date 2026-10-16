//! One generation and one fetch at a time on the host (IMG-005, IMG-011).
//!
//! A kit can run several agent processes on one Mac, all sharing the
//! generator's directory. Each process already serializes its own work; an
//! advisory lock file under that directory serializes the processes, so two
//! never load a model at once or write the same download.

// Shared by the generator's modules only; plain `pub` here would trip
// `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::fs::File;
use std::path::Path;
use std::time::Duration;

use tokio::time::Instant;

use crate::error::{ImageGenError, Result};

/// How often a waiting process tries the lock again.
const RETRY: Duration = Duration::from_millis(250);

/// The lock file serializing generation on the host.
pub(crate) const GENERATE: &str = ".generate.lock";

/// The lock file serializing downloads into the store.
pub(crate) const FETCH: &str = ".fetch.lock";

/// Held while this process generates or fetches; released when dropped,
/// or when the process exits however it exits.
#[derive(Debug)]
pub(crate) struct HostLock {
    _file: File,
}

impl HostLock {
    /// Wait for the lock `name` under `dir`, until `deadline` when one is
    /// given. `waiting` runs once if another holder has it: a request that
    /// waits without saying so looks stalled to its delegator (ASSET-009).
    pub(crate) async fn acquire(
        dir: &Path,
        name: &str,
        deadline: Option<Instant>,
        waiting: impl FnOnce(),
    ) -> Result<Self> {
        tokio::fs::create_dir_all(dir).await?;
        let path = dir.join(name);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        let mut waiting = Some(waiting);
        loop {
            if try_lock(&file)? {
                return Ok(Self { _file: file });
            }
            if let Some(waiting) = waiting.take() {
                waiting();
            }
            if deadline.is_some_and(|d| Instant::now() + RETRY >= d) {
                return Err(ImageGenError::TimedOut);
            }
            tokio::time::sleep(RETRY).await;
        }
    }
}

/// Take the exclusive lock if no other open file holds it.
#[cfg(unix)]
fn try_lock(file: &File) -> Result<bool> {
    use std::os::fd::AsRawFd;
    // SAFETY: `file` is open for the duration of the call.
    let taken = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
    if taken {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(false)
    } else {
        Err(error.into())
    }
}

/// Hosts without flock never generate (IMG-001); the lock is theirs at once.
#[cfg(not(unix))]
fn try_lock(_file: &File) -> Result<bool> {
    Ok(true)
}

#[cfg(test)]
mod tests {
    // `#[tokio::test]` expands to `Runtime::block_on`, which the workspace
    // lint set disallows in library code.
    #![allow(clippy::disallowed_methods)]

    use super::*;

    /// IMG-011: a second holder waits for the first, across open files as
    /// across processes, says once that it is waiting, and gives up at its
    /// deadline.
    #[arkavo_test_macros::spec("IMG-011")]
    #[tokio::test]
    async fn a_second_holder_waits_for_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let free = std::cell::Cell::new(0);
        let first = HostLock::acquire(dir.path(), GENERATE, None, || free.set(free.get() + 1))
            .await
            .unwrap();
        assert_eq!(free.get(), 0, "a free lock is taken without waiting");
        let soon = Instant::now() + Duration::from_millis(600);
        let held = std::cell::Cell::new(0);
        let waited = HostLock::acquire(dir.path(), GENERATE, Some(soon), || {
            held.set(held.get() + 1);
        })
        .await;
        assert!(matches!(waited, Err(ImageGenError::TimedOut)), "{waited:?}");
        assert_eq!(held.get(), 1, "told once, however long it waits");

        let other = HostLock::acquire(dir.path(), FETCH, None, || {}).await;
        assert!(other.is_ok(), "locks with other names are independent");

        let path = dir.path().to_path_buf();
        let second =
            tokio::spawn(async move { HostLock::acquire(&path, GENERATE, None, || {}).await });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!second.is_finished(), "still held by the first");
        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(5), second)
            .await
            .expect("taken once released")
            .unwrap();
        assert!(second.is_ok());
    }
}
