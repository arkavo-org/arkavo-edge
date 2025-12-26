//! Adaptive polling infrastructure for MCP clients
//!
//! This module provides intelligent polling with:
//! - Dual-signal adaptive backoff (empty results + latency monitoring)
//! - Rate limiting with jitter to prevent thundering herd
//! - Configurable warmup and baseline window for latency analysis

use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::time::Duration;

/// Configuration for polling behavior
#[derive(Clone, Debug)]
pub struct PollConfig {
    /// Minimum polling interval (fastest rate)
    pub min_interval: Duration,
    /// Maximum polling interval (slowest rate after backoff)
    pub max_interval: Duration,
    /// Multiplier for exponential backoff
    pub backoff_factor: f32,
    /// Consecutive empty results before backing off
    pub backoff_threshold: usize,
    /// Jitter factor (0.0 to 1.0) to randomize intervals
    pub jitter_factor: f32,
    /// Maximum polls per second across all endpoints
    pub max_polls_per_second: f32,
    /// Number of polls to skip before establishing latency baseline
    pub warmup_count: usize,
    /// Rolling window size for latency baseline calculation
    pub baseline_window: usize,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            min_interval: Duration::from_millis(100),
            max_interval: Duration::from_secs(10),
            backoff_factor: 2.0,
            backoff_threshold: 3,
            jitter_factor: 0.1,
            max_polls_per_second: 10.0,
            warmup_count: 5,
            baseline_window: 10,
        }
    }
}

/// Endpoint registered for polling
#[derive(Clone, Debug)]
pub struct PollableEndpoint {
    /// MCP method to call (e.g., "tools/call")
    pub method: String,
    /// Parameters for the method call
    pub params: Option<Value>,
}

/// Normalized poll result notification parameters
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PollResultParams {
    /// Server name that was polled
    pub server: String,
    /// Original endpoint/method that was polled
    pub endpoint: String,
    /// Result from the poll
    pub result: Value,
    /// Response latency in milliseconds
    pub latency_ms: u64,
    /// True if this came from polling, false if from push
    pub is_poll: bool,
}

/// Adaptive backoff manager using dual-signal approach
///
/// Uses two independent signals to determine polling rate:
/// 1. Empty results: Backs off when consecutive polls return no data
/// 2. Latency monitoring: Backs off when server response times increase
pub struct AdaptiveBackoff {
    config: PollConfig,
    current_interval: Duration,
    consecutive_empty: usize,
    poll_count: usize,
    baseline_samples: VecDeque<Duration>,
    baseline_response_time: Option<Duration>,
    latency_backoff_interval: Duration,
}

impl AdaptiveBackoff {
    /// Create a new adaptive backoff manager with the given configuration
    pub fn new(config: PollConfig) -> Self {
        let min_interval = config.min_interval;
        Self {
            config,
            current_interval: min_interval,
            consecutive_empty: 0,
            poll_count: 0,
            baseline_samples: VecDeque::new(),
            baseline_response_time: None,
            latency_backoff_interval: min_interval,
        }
    }

    /// Record a poll result and update backoff state
    pub fn record(&mut self, latency: Duration, has_data: bool) {
        self.poll_count += 1;

        // Update latency baseline (skip warmup period)
        if self.poll_count > self.config.warmup_count {
            self.update_latency_baseline(latency);
        }

        // Update empty-result backoff
        if has_data {
            self.consecutive_empty = 0;
            self.current_interval = self.config.min_interval;
        } else {
            self.consecutive_empty += 1;
            if self.consecutive_empty >= self.config.backoff_threshold {
                self.increase_interval();
            }
        }
    }

    /// Record an error (treat as empty + latency spike)
    pub fn record_error(&mut self) {
        self.consecutive_empty += 1;
        if self.consecutive_empty >= self.config.backoff_threshold {
            self.increase_interval();
        }
        // Also increase latency-based backoff on errors
        self.latency_backoff_interval = Duration::from_secs_f32(
            (self.latency_backoff_interval.as_secs_f32() * self.config.backoff_factor)
                .min(self.config.max_interval.as_secs_f32()),
        );
    }

    /// Get the next polling interval (applies jitter)
    pub fn next_interval(&self) -> Duration {
        // Use the slower of the two signals
        let base_interval = self.current_interval.max(self.latency_backoff_interval);
        let capped = base_interval.min(self.config.max_interval);
        self.add_jitter(capped)
    }

    /// Get the minimum cycle time based on rate limit
    pub fn min_cycle_time(&self) -> Duration {
        Duration::from_secs_f32(1.0 / self.config.max_polls_per_second)
    }

    /// Reset backoff state (e.g., after successful reconnection)
    pub fn reset(&mut self) {
        self.current_interval = self.config.min_interval;
        self.consecutive_empty = 0;
        self.latency_backoff_interval = self.config.min_interval;
        self.baseline_samples.clear();
        self.baseline_response_time = None;
        self.poll_count = 0;
    }

    fn update_latency_baseline(&mut self, latency: Duration) {
        // Add to rolling window
        self.baseline_samples.push_back(latency);
        if self.baseline_samples.len() > self.config.baseline_window {
            self.baseline_samples.pop_front();
        }

        // Compute median baseline once we have enough samples
        if self.baseline_samples.len() >= self.config.baseline_window / 2 {
            self.baseline_response_time = Some(self.compute_median());
        }

        // Apply latency-based backoff
        if let Some(baseline) = self.baseline_response_time {
            let ratio = latency.as_secs_f32() / baseline.as_secs_f32();
            if ratio > 4.0 {
                // Server severely stressed - jump to max
                self.latency_backoff_interval = self.config.max_interval;
            } else if ratio > 2.0 {
                // Server stressed - increase interval
                self.latency_backoff_interval = Duration::from_secs_f32(
                    (self.latency_backoff_interval.as_secs_f32() * self.config.backoff_factor)
                        .min(self.config.max_interval.as_secs_f32()),
                );
            } else if ratio < 1.2 {
                // Response time healthy - allow faster polling
                self.latency_backoff_interval = self.config.min_interval;
            }
        }
    }

    fn compute_median(&self) -> Duration {
        let mut sorted: Vec<Duration> = self.baseline_samples.iter().copied().collect();
        sorted.sort();
        let mid = sorted.len() / 2;
        if sorted.len().is_multiple_of(2) {
            Duration::from_secs_f32(f32::midpoint(
                sorted[mid - 1].as_secs_f32(),
                sorted[mid].as_secs_f32(),
            ))
        } else {
            sorted[mid]
        }
    }

    fn increase_interval(&mut self) {
        self.current_interval = Duration::from_secs_f32(
            (self.current_interval.as_secs_f32() * self.config.backoff_factor)
                .min(self.config.max_interval.as_secs_f32()),
        );
    }

    fn add_jitter(&self, interval: Duration) -> Duration {
        let mut rng = rand::thread_rng();
        let jitter_range = interval.as_secs_f32() * self.config.jitter_factor;
        let jitter = rng.gen_range(-jitter_range..=jitter_range);
        Duration::from_secs_f32((interval.as_secs_f32() + jitter).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = PollConfig::default();
        assert_eq!(config.min_interval, Duration::from_millis(100));
        assert_eq!(config.max_interval, Duration::from_secs(10));
        assert!((config.backoff_factor - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_backoff_on_empty_results() {
        let config = PollConfig {
            backoff_threshold: 2,
            ..Default::default()
        };
        let mut backoff = AdaptiveBackoff::new(config);

        // First empty - no backoff yet
        backoff.record(Duration::from_millis(50), false);
        assert_eq!(backoff.consecutive_empty, 1);

        // Second empty - triggers backoff
        backoff.record(Duration::from_millis(50), false);
        assert!(backoff.current_interval > Duration::from_millis(100));
    }

    #[test]
    fn test_reset_on_data() {
        let config = PollConfig {
            backoff_threshold: 1,
            ..Default::default()
        };
        let mut backoff = AdaptiveBackoff::new(config);

        // Trigger backoff
        backoff.record(Duration::from_millis(50), false);
        assert!(backoff.current_interval > Duration::from_millis(100));

        // Reset on data
        backoff.record(Duration::from_millis(50), true);
        assert_eq!(backoff.current_interval, Duration::from_millis(100));
        assert_eq!(backoff.consecutive_empty, 0);
    }

    #[test]
    fn test_interval_capped_at_max() {
        let config = PollConfig {
            backoff_threshold: 1,
            max_interval: Duration::from_secs(1),
            ..Default::default()
        };
        let mut backoff = AdaptiveBackoff::new(config);

        // Many empties
        for _ in 0..20 {
            backoff.record(Duration::from_millis(50), false);
        }

        let interval = backoff.next_interval();
        // Should be around max_interval (with jitter)
        assert!(interval <= Duration::from_millis(1100));
    }

    #[test]
    fn test_min_cycle_time() {
        let config = PollConfig {
            max_polls_per_second: 5.0,
            ..Default::default()
        };
        let backoff = AdaptiveBackoff::new(config);
        let cycle_time = backoff.min_cycle_time();
        // Use approximate comparison due to floating point imprecision
        assert!(cycle_time >= Duration::from_millis(199));
        assert!(cycle_time <= Duration::from_millis(201));
    }

    #[test]
    fn test_reset() {
        let config = PollConfig {
            backoff_threshold: 1,
            ..Default::default()
        };
        let mut backoff = AdaptiveBackoff::new(config);

        // Trigger backoff
        backoff.record(Duration::from_millis(50), false);
        assert!(backoff.current_interval > Duration::from_millis(100));

        // Reset
        backoff.reset();
        assert_eq!(backoff.current_interval, Duration::from_millis(100));
        assert_eq!(backoff.consecutive_empty, 0);
        assert_eq!(backoff.poll_count, 0);
    }
}
