//! The time a compute budget measures its window against.

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

/// Where a budget reads the time.
///
/// A window lasts minutes, and what matters about it is what happens when it
/// ends. Reading the time through this lets a test end a window without
/// waiting for it.
#[derive(Clone)]
pub struct BudgetClock(Arc<dyn Fn() -> Instant + Send + Sync>);

impl BudgetClock {
    /// The machine's monotonic clock.
    #[must_use]
    pub fn system() -> Self {
        Self(Arc::new(Instant::now))
    }

    /// A clock that reads whatever `now` returns.
    pub fn new(now: impl Fn() -> Instant + Send + Sync + 'static) -> Self {
        Self(Arc::new(now))
    }

    pub(super) fn now(&self) -> Instant {
        (self.0)()
    }
}

impl Default for BudgetClock {
    fn default() -> Self {
        Self::system()
    }
}

impl fmt::Debug for BudgetClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BudgetClock")
    }
}
