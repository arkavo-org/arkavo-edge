//! R6: Per-provider circuit breaker for LLM calls.
//!
//! A simple three-state breaker (Closed / Open / HalfOpen) that tracks
//! recent failures of a named provider (e.g., "openai", "anthropic",
//! "llama-cpp-local") and short-circuits calls when a provider is
//! demonstrably unhealthy. Prevents the orchestrator from wasting
//! budget and wall-clock on requests that are almost certain to fail.
//!
//! ## State machine
//!
//! * **Closed** (normal): requests flow through. Consecutive failures
//!   are counted; on reaching `failure_threshold`, transition to Open.
//! * **Open** (tripped): all requests are rejected immediately for
//!   `cooldown` duration. After the cooldown expires, transition to
//!   HalfOpen on the next request.
//! * **HalfOpen** (probing): one trial request is allowed. If it
//!   succeeds, transition back to Closed; if it fails, return to Open
//!   with a fresh cooldown.
//!
//! ## Usage
//!
//! ```ignore
//! let breaker = Arc::new(CircuitBreaker::default());
//! match breaker.guard("openai") {
//!     Ok(permit) => {
//!         match provider.call().await {
//!             Ok(r) => { permit.success(); Ok(r) }
//!             Err(e) => { permit.failure(); Err(e) }
//!         }
//!     }
//!     Err(BreakerError::Open { retry_after }) => {
//!         // Skip this provider; try another or bail.
//!     }
//! }
//! ```
//!
//! The breaker is safe to share across tasks via `Arc` and uses only
//! internal `Mutex` locks — no async in the hot path.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default consecutive-failure threshold before tripping.
pub const DEFAULT_FAILURE_THRESHOLD: u32 = 5;

/// Default cooldown between Open and HalfOpen.
pub const DEFAULT_COOLDOWN: Duration = Duration::from_secs(30);

/// Default success threshold in HalfOpen before closing.
pub const DEFAULT_SUCCESS_THRESHOLD: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Closed,
    Open { until: Instant },
    HalfOpen,
}

#[derive(Debug)]
struct ProviderState {
    state: State,
    consecutive_failures: u32,
    half_open_successes: u32,
    last_failure_at: Option<Instant>,
    last_success_at: Option<Instant>,
}

impl Default for ProviderState {
    fn default() -> Self {
        Self {
            state: State::Closed,
            consecutive_failures: 0,
            half_open_successes: 0,
            last_failure_at: None,
            last_success_at: None,
        }
    }
}

/// Per-provider circuit breaker.
pub struct CircuitBreaker {
    providers: Mutex<HashMap<String, ProviderState>>,
    failure_threshold: u32,
    success_threshold: u32,
    cooldown: Duration,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(
            DEFAULT_FAILURE_THRESHOLD,
            DEFAULT_SUCCESS_THRESHOLD,
            DEFAULT_COOLDOWN,
        )
    }
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, success_threshold: u32, cooldown: Duration) -> Self {
        Self {
            providers: Mutex::new(HashMap::new()),
            failure_threshold,
            success_threshold,
            cooldown,
        }
    }

    /// Attempt to reserve a call against `provider`. Returns either a
    /// permit (call allowed) or [`BreakerError::Open`] with the time the
    /// caller should wait before retrying.
    pub fn guard<'a>(&'a self, provider: &str) -> Result<Permit<'a>, BreakerError> {
        let now = Instant::now();
        let mut guard = self.providers.lock().expect("circuit breaker poisoned");
        let entry = guard.entry(provider.to_string()).or_default();

        match entry.state {
            State::Closed => {
                // Allow through.
            }
            State::Open { until } => {
                if now >= until {
                    // Cooldown elapsed; promote to HalfOpen and allow one probe.
                    entry.state = State::HalfOpen;
                    entry.half_open_successes = 0;
                } else {
                    return Err(BreakerError::Open {
                        retry_after: until.saturating_duration_since(now),
                    });
                }
            }
            State::HalfOpen => {
                // Allow the probe through; permit records the outcome.
            }
        }

        Ok(Permit {
            breaker: self,
            provider: provider.to_string(),
        })
    }

    fn record_success(&self, provider: &str) {
        let mut guard = self.providers.lock().expect("circuit breaker poisoned");
        let entry = guard.entry(provider.to_string()).or_default();
        entry.consecutive_failures = 0;
        entry.last_success_at = Some(Instant::now());
        match entry.state {
            State::Closed => {}
            State::HalfOpen => {
                entry.half_open_successes += 1;
                if entry.half_open_successes >= self.success_threshold {
                    entry.state = State::Closed;
                    entry.half_open_successes = 0;
                }
            }
            State::Open { .. } => {
                // Success recorded while Open means we were probably in
                // HalfOpen transitioning; treat as a success.
                entry.state = State::Closed;
            }
        }
    }

    fn record_failure(&self, provider: &str) {
        let mut guard = self.providers.lock().expect("circuit breaker poisoned");
        let entry = guard.entry(provider.to_string()).or_default();
        entry.consecutive_failures += 1;
        entry.last_failure_at = Some(Instant::now());
        let should_trip = match entry.state {
            State::Closed => entry.consecutive_failures >= self.failure_threshold,
            State::HalfOpen => true, // single failure during probe re-opens.
            State::Open { .. } => false,
        };
        if should_trip {
            entry.state = State::Open {
                until: Instant::now() + self.cooldown,
            };
            entry.half_open_successes = 0;
        }
    }

    /// Inspect the current state for diagnostics / metrics.
    pub fn snapshot(&self, provider: &str) -> BreakerSnapshot {
        let guard = self.providers.lock().expect("circuit breaker poisoned");
        let entry = guard.get(provider);
        match entry {
            None => BreakerSnapshot {
                state_label: "closed",
                consecutive_failures: 0,
                retry_after: None,
            },
            Some(s) => {
                let (label, retry_after) = match s.state {
                    State::Closed => ("closed", None),
                    State::Open { until } => (
                        "open",
                        Some(until.saturating_duration_since(Instant::now())),
                    ),
                    State::HalfOpen => ("half-open", None),
                };
                BreakerSnapshot {
                    state_label: label,
                    consecutive_failures: s.consecutive_failures,
                    retry_after,
                }
            }
        }
    }
}

/// A permit held by an in-flight call. On drop without an explicit
/// outcome, the call is assumed to have failed (fail-safe): this
/// avoids silently leaving the breaker in a stale state if a caller
/// returns early on an unexpected code path. Call [`Permit::success`]
/// or [`Permit::failure`] explicitly for clarity.
pub struct Permit<'a> {
    breaker: &'a CircuitBreaker,
    provider: String,
}

impl<'a> Permit<'a> {
    /// Mark the guarded call as successful.
    pub fn success(self) {
        self.breaker.record_success(&self.provider);
        std::mem::forget(self);
    }

    /// Mark the guarded call as failed. This counts toward the
    /// failure threshold and may trip the breaker.
    pub fn failure(self) {
        self.breaker.record_failure(&self.provider);
        std::mem::forget(self);
    }
}

impl<'a> Drop for Permit<'a> {
    fn drop(&mut self) {
        // Treat implicit drops as failures; callers who want to record
        // success must do so explicitly via `permit.success()`. This
        // matches the fail-safe posture of the breaker.
        self.breaker.record_failure(&self.provider);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerError {
    /// Breaker is tripped; do not call the provider.
    Open { retry_after: Duration },
}

impl std::fmt::Display for BreakerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open { retry_after } => write!(
                f,
                "circuit breaker open; retry after {}s",
                retry_after.as_secs()
            ),
        }
    }
}

impl std::error::Error for BreakerError {}

/// Read-only view of a breaker's state for one provider.
#[derive(Debug, Clone, Copy)]
pub struct BreakerSnapshot {
    pub state_label: &'static str,
    pub consecutive_failures: u32,
    pub retry_after: Option<Duration>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_allows_calls() {
        let b = CircuitBreaker::default();
        let permit = b.guard("p").unwrap();
        permit.success();
        assert_eq!(b.snapshot("p").state_label, "closed");
    }

    #[test]
    fn trips_after_threshold_failures() {
        let b = CircuitBreaker::new(3, 1, Duration::from_secs(60));
        for _ in 0..3 {
            b.guard("p").unwrap().failure();
        }
        let err = b.guard("p").unwrap_err();
        match err {
            BreakerError::Open { retry_after } => {
                assert!(retry_after.as_secs() > 0);
            }
        }
        assert_eq!(b.snapshot("p").state_label, "open");
    }

    #[test]
    fn success_resets_failure_count() {
        let b = CircuitBreaker::new(3, 1, Duration::from_secs(60));
        b.guard("p").unwrap().failure();
        b.guard("p").unwrap().failure();
        b.guard("p").unwrap().success();
        // Back to zero; another 2 failures should NOT trip.
        b.guard("p").unwrap().failure();
        b.guard("p").unwrap().failure();
        assert!(b.guard("p").is_ok());
    }

    #[test]
    fn half_open_success_closes_breaker() {
        let b = CircuitBreaker::new(2, 1, Duration::from_millis(10));
        b.guard("p").unwrap().failure();
        b.guard("p").unwrap().failure();
        // Now Open. Wait out the cooldown.
        std::thread::sleep(Duration::from_millis(20));
        // Next guard should transition to HalfOpen and allow the probe.
        let permit = b.guard("p").unwrap();
        permit.success();
        assert_eq!(b.snapshot("p").state_label, "closed");
    }

    #[test]
    fn half_open_failure_reopens() {
        let b = CircuitBreaker::new(2, 1, Duration::from_millis(10));
        b.guard("p").unwrap().failure();
        b.guard("p").unwrap().failure();
        std::thread::sleep(Duration::from_millis(20));
        // Probe fails; breaker should re-open.
        b.guard("p").unwrap().failure();
        let snap = b.snapshot("p");
        assert_eq!(snap.state_label, "open");
    }

    #[test]
    fn providers_are_independent() {
        let b = CircuitBreaker::new(2, 1, Duration::from_secs(60));
        b.guard("p1").unwrap().failure();
        b.guard("p1").unwrap().failure();
        // p1 tripped; p2 unaffected.
        assert!(b.guard("p1").is_err());
        assert!(b.guard("p2").is_ok());
    }

    #[test]
    fn permit_dropped_implicitly_counts_as_failure() {
        let b = CircuitBreaker::new(2, 1, Duration::from_secs(60));
        {
            let _permit = b.guard("p").unwrap();
            // drop without calling success/failure
        }
        {
            let _permit = b.guard("p").unwrap();
        }
        assert!(b.guard("p").is_err(), "breaker should have tripped");
    }
}
