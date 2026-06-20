//! Global named-event counters for router / telemetry events.
//!
//! Components increment a counter by event kind at emit time; a metrics
//! snapshot reads the totals. This is the backend sink for `RouterEvent`s
//! (`cloud_escalation_blocked`, `local_feasibility`, …) so they are observable
//! without a polling consumer. Cheap (one `RwLock<HashMap>` behind a global)
//! and pull-based, mirroring [`crate::subsystem_timing`].

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// Monotonic counters keyed by event kind.
#[derive(Debug, Default)]
pub struct EventCounters {
    counts: RwLock<HashMap<String, u64>>,
}

impl EventCounters {
    /// Increment the counter for `kind` by one.
    pub fn increment(&self, kind: &str) {
        if let Ok(mut counts) = self.counts.write() {
            *counts.entry(kind.to_string()).or_insert(0) += 1;
        }
    }

    /// Current total for `kind` (0 if never incremented).
    pub fn count(&self, kind: &str) -> u64 {
        self.counts
            .read()
            .ok()
            .and_then(|c| c.get(kind).copied())
            .unwrap_or(0)
    }

    /// Snapshot of all counters for a metrics export.
    pub fn snapshot(&self) -> HashMap<String, u64> {
        self.counts.read().map(|c| c.clone()).unwrap_or_default()
    }
}

static GLOBAL: LazyLock<EventCounters> = LazyLock::new(EventCounters::default);

/// Process-wide event counters.
pub fn global_event_counters() -> &'static EventCounters {
    &GLOBAL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increment_and_read() {
        let c = EventCounters::default();
        assert_eq!(c.count("x"), 0);
        c.increment("x");
        c.increment("x");
        c.increment("y");
        assert_eq!(c.count("x"), 2);
        assert_eq!(c.count("y"), 1);
        let snap = c.snapshot();
        assert_eq!(snap.get("x"), Some(&2));
        assert_eq!(snap.get("y"), Some(&1));
    }

    #[test]
    fn global_is_shared() {
        global_event_counters().increment("global_test_kind");
        assert!(global_event_counters().count("global_test_kind") >= 1);
    }
}
