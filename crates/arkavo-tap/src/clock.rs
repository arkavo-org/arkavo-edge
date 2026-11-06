//! Time, injected, so signature windows and replay retention can be tested at their edges.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// UTC time in whole seconds since the Unix epoch, the unit of `created` and `expires`.
pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
}

/// The host clock. Hosts must be kept synchronized: the strict profile allows no future skew.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

/// A clock that only moves when told to. Clones share the same time.
#[derive(Debug, Clone)]
pub struct ManualClock(Arc<AtomicI64>);

impl ManualClock {
    pub fn new(now: i64) -> Self {
        ManualClock(Arc::new(AtomicI64::new(now)))
    }

    pub fn set(&self, now: i64) {
        self.0.store(now, Ordering::SeqCst);
    }

    pub fn advance(&self, seconds: i64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl<C: Clock + ?Sized> Clock for Arc<C> {
    fn now(&self) -> i64 {
        (**self).now()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_after_2025() {
        assert!(SystemClock.now() > 1_735_689_600);
    }

    #[test]
    fn manual_clock_clones_share_time() {
        let a = ManualClock::new(100);
        let b = a.clone();
        a.advance(20);
        assert_eq!(b.now(), 120);
        b.set(5);
        assert_eq!(a.now(), 5);
        let shared: Arc<dyn Clock> = Arc::new(a);
        assert_eq!(shared.now(), 5);
    }
}
