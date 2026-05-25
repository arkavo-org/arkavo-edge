//! Per-model rolling NGRAM spec-decoding accept-rate.
//!
//! The router uses this to decide whether to enable spec for the next
//! request to a given model. Below a threshold (15% over 20 requests),
//! spec is auto-skipped for that model and a structured event is emitted.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

pub struct SpecStats {
    window: u32,
    threshold_pct: u32,
    inner: Mutex<HashMap<String, ModelEntry>>,
}

struct ModelEntry {
    samples: VecDeque<(u32, u32)>,
    notified_low: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpecDecision {
    pub use_spec: bool,
    /// Set once per below-threshold crossing; `Some(rate_pct)` on the first
    /// call after the window fills with low-acceptance data.
    pub crossed_below_threshold: Option<u32>,
}

impl Default for SpecStats {
    fn default() -> Self {
        Self::new(20, 15)
    }
}

impl SpecStats {
    pub fn new(window: u32, threshold_pct: u32) -> Self {
        Self {
            window,
            threshold_pct,
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// Configured rolling window size (number of recent requests considered).
    pub fn window(&self) -> u32 {
        self.window
    }

    /// Record a spec-decoding sample for `model`.
    ///
    /// Only call this when `spec_bypassed` is `None` on the `InferenceTiming` —
    /// bypassed paths give no quality signal for this model.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned (another thread panicked while
    /// holding the lock). This is unrecoverable in production.
    pub fn record(&self, model: &str, n_draft: u32, n_accepted: u32) {
        let mut g = self.inner.lock().expect("spec_stats poisoned");
        let entry = g.entry(model.to_string()).or_insert_with(|| ModelEntry {
            samples: VecDeque::with_capacity(self.window as usize),
            notified_low: false,
        });
        entry.samples.push_back((n_draft, n_accepted));
        if entry.samples.len() > self.window as usize {
            entry.samples.pop_front();
        }
    }

    /// Decide whether the next request to `model` should use spec decoding.
    ///
    /// Returns `use_spec = true` (optimistic) until the rolling window fills.
    /// Sets `crossed_below_threshold` exactly once per low→below transition;
    /// re-arms when the model recovers above threshold.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned (another thread panicked while
    /// holding the lock). This is unrecoverable in production.
    pub fn decide(&self, model: &str) -> SpecDecision {
        let mut g = self.inner.lock().expect("spec_stats poisoned");
        let Some(entry) = g.get_mut(model) else {
            return SpecDecision {
                use_spec: true,
                crossed_below_threshold: None,
            };
        };
        if entry.samples.len() < self.window as usize {
            return SpecDecision {
                use_spec: true,
                crossed_below_threshold: None,
            };
        }
        let (sum_draft, sum_acc): (u32, u32) = entry
            .samples
            .iter()
            .fold((0, 0), |(d, a), (nd, na)| (d + nd, a + na));
        let rate_pct = (sum_acc * 100).checked_div(sum_draft).unwrap_or(0);

        let above = rate_pct >= self.threshold_pct;
        let crossed = if !above && !entry.notified_low {
            entry.notified_low = true;
            Some(rate_pct)
        } else {
            if above && entry.notified_low {
                entry.notified_low = false;
            }
            None
        };
        SpecDecision {
            use_spec: above,
            crossed_below_threshold: crossed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_model_enables_spec() {
        let s = SpecStats::new(20, 15);
        let d = s.decide("nope");
        assert!(d.use_spec);
        assert!(d.crossed_below_threshold.is_none());
    }

    #[test]
    fn insufficient_samples_enables_spec() {
        let s = SpecStats::new(20, 15);
        for _ in 0..5 {
            s.record("m", 10, 0);
        }
        let d = s.decide("m");
        assert!(d.use_spec);
    }

    #[test]
    fn low_accept_rate_disables_and_signals_once() {
        let s = SpecStats::new(20, 15);
        for _ in 0..20 {
            s.record("m", 10, 0);
        }
        let d1 = s.decide("m");
        assert!(!d1.use_spec);
        assert_eq!(d1.crossed_below_threshold, Some(0));
        let d2 = s.decide("m");
        assert!(!d2.use_spec);
        assert_eq!(d2.crossed_below_threshold, None);
    }

    #[test]
    fn high_accept_rate_keeps_spec_on() {
        let s = SpecStats::new(20, 15);
        for _ in 0..20 {
            s.record("m", 10, 5);
        }
        let d = s.decide("m");
        assert!(d.use_spec);
        assert!(d.crossed_below_threshold.is_none());
    }

    #[test]
    fn recovery_rearms_signal() {
        let s = SpecStats::new(20, 15);
        for _ in 0..20 {
            s.record("m", 10, 0);
        }
        // First crossing: signal fires
        assert!(s.decide("m").crossed_below_threshold.is_some());
        // Recover above threshold — a decide() call during recovery resets notified_low
        for _ in 0..20 {
            s.record("m", 10, 8);
        }
        let recovered = s.decide("m");
        assert!(recovered.use_spec, "rate recovered, spec should be enabled");
        assert!(
            recovered.crossed_below_threshold.is_none(),
            "no signal during recovery"
        );
        // Drop back below threshold — signal must fire again
        for _ in 0..20 {
            s.record("m", 10, 0);
        }
        let d = s.decide("m");
        assert!(!d.use_spec);
        assert!(d.crossed_below_threshold.is_some());
    }
}
