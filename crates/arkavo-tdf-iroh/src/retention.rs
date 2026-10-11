//! Blobs staged for another party: kept for their retention window, then
//! untagged for the garbage collector, within a bound on staged bytes
//! (IROH-009, IROH-010).

// The node and the transport share this state inside the crate; plain `pub`
// in a private module would trip `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use iroh_blobs::store::mem::MemStore;
use tracing::{debug, warn};

use crate::error::IrohError;

const TAG_PREFIX: &str = "arkavo/retain/";

#[derive(Debug)]
struct Retained {
    tag: String,
    bytes: u64,
    expires: Instant,
}

/// The retained blobs of one node.
#[derive(Debug)]
pub(crate) struct Retention {
    limit: u64,
    next: AtomicU64,
    entries: Mutex<Vec<Retained>>,
}

impl Retention {
    pub(crate) const fn new(limit: u64) -> Self {
        Self {
            limit,
            next: AtomicU64::new(0),
            entries: Mutex::new(Vec::new()),
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, Vec<Retained>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Count `bytes` against the bound for `retention` and name the tag to
    /// stage them under. Blobs still in their window are never evicted.
    pub(crate) fn reserve(&self, bytes: u64, retention: Duration) -> Result<String, IrohError> {
        let mut entries = self.entries();
        let staged: u64 = entries.iter().map(|e| e.bytes).sum();
        if staged.saturating_add(bytes) > self.limit {
            return Err(IrohError::StoreFull {
                staged,
                requested: bytes,
                limit: self.limit,
            });
        }
        let tag = format!("{TAG_PREFIX}{}", self.next.fetch_add(1, Ordering::Relaxed));
        entries.push(Retained {
            tag: tag.clone(),
            bytes,
            expires: Instant::now() + retention,
        });
        drop(entries);
        Ok(tag)
    }

    /// Forget a reservation whose blob was never staged.
    pub(crate) fn release(&self, tag: &str) {
        self.entries().retain(|e| e.tag != tag);
    }

    /// Remove entries whose window has passed, returning their tags.
    pub(crate) fn expired(&self, now: Instant) -> Vec<String> {
        let mut entries = self.entries();
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut *entries)
            .into_iter()
            .partition(|e| e.expires <= now);
        *entries = kept;
        drop(entries);
        gone.into_iter().map(|e| e.tag).collect()
    }

    pub(crate) fn staged_bytes(&self) -> u64 {
        self.entries().iter().map(|e| e.bytes).sum()
    }
}

/// Untag every expired blob; the store's garbage collector then frees it.
pub(crate) async fn sweep(store: &MemStore, retention: &Retention) {
    for tag in retention.expired(Instant::now()) {
        match store.tags().delete(&tag).await {
            Ok(_) => debug!(%tag, "retention ended"),
            Err(e) => warn!(%tag, error = %e, "could not untag expired blob"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservations_count_against_the_bound() {
        let retention = Retention::new(100);
        let a = retention.reserve(60, Duration::from_mins(1)).unwrap();
        let err = retention.reserve(50, Duration::from_mins(1)).unwrap_err();
        assert!(matches!(
            err,
            IrohError::StoreFull {
                staged: 60,
                requested: 50,
                limit: 100
            }
        ));
        assert!(retention.reserve(40, Duration::from_mins(1)).is_ok());
        retention.release(&a);
        assert_eq!(retention.staged_bytes(), 40);
        assert!(a.starts_with(TAG_PREFIX));
    }

    #[test]
    fn expired_entries_are_returned_once() {
        let retention = Retention::new(1000);
        let short = retention.reserve(10, Duration::ZERO).unwrap();
        let long = retention.reserve(10, Duration::from_hours(1)).unwrap();
        assert_ne!(short, long);
        assert_eq!(retention.expired(Instant::now()), [short]);
        assert!(retention.expired(Instant::now()).is_empty());
        assert_eq!(retention.staged_bytes(), 10);
    }
}
