use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Lightweight metrics snapshot for WebSocket streaming to AG-UI
/// Target: ≤ 1KB JSON serialized size
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// Timestamp of this snapshot
    pub timestamp: DateTime<Utc>,
    /// Active session count
    pub active_sessions: u64,
    /// Messages per second (rate)
    pub messages_per_second: f64,
    /// Deltas per second (rate)
    pub deltas_per_second: f64,
    /// P95 response latency in milliseconds
    pub p95_latency_ms: f64,
    /// P99 response latency in milliseconds
    pub p99_latency_ms: f64,
    /// Error rate (errors per second)
    pub error_rate: f64,
    /// Memory usage in MB
    pub memory_usage_mb: f64,
    /// CPU usage percentage (0-100)
    pub cpu_usage_percent: f64,
    /// Session state breakdown
    pub session_states: SessionStateBreakdown,
    /// Top error types (limited to 5 most common)
    pub top_errors: Vec<ErrorCount>,
}

/// Breakdown of sessions by state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStateBreakdown {
    pub active: u64,
}

/// Error count for top errors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorCount {
    pub error_type: String,
    pub count: u64,
}

impl MetricsSnapshot {
    /// Create a new empty metrics snapshot
    pub fn new() -> Self {
        Self {
            timestamp: Utc::now(),
            active_sessions: 0,
            messages_per_second: 0.0,
            deltas_per_second: 0.0,
            p95_latency_ms: 0.0,
            p99_latency_ms: 0.0,
            error_rate: 0.0,
            memory_usage_mb: 0.0,
            cpu_usage_percent: 0.0,
            session_states: SessionStateBreakdown { active: 0 },
            top_errors: Vec::new(),
        }
    }

    /// Estimate JSON serialized size in bytes
    pub fn estimated_json_size(&self) -> usize {
        serde_json::to_string(self).map(|s| s.len()).unwrap_or(0)
    }

    /// Validate that snapshot size is within acceptable limits
    pub fn validate_size(&self) -> Result<(), String> {
        let size = self.estimated_json_size();
        if size > 1024 {
            Err(format!(
                "MetricsSnapshot too large: {size} bytes (max 1024)"
            ))
        } else {
            Ok(())
        }
    }
}

impl Default for MetricsSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

/// Sampler configuration for metrics collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSamplerConfig {
    /// How often to sample metrics (seconds)
    pub sample_interval_seconds: u64,
    /// Maximum number of top errors to include
    pub max_error_types: usize,
    /// Enable memory/CPU usage collection
    pub enable_system_metrics: bool,
}

impl Default for MetricsSamplerConfig {
    fn default() -> Self {
        Self {
            sample_interval_seconds: 1,
            max_error_types: 5,
            enable_system_metrics: true,
        }
    }
}

/// Metrics sampler that collects and aggregates metrics into snapshots
pub struct MetricsSampler {
    pub config: MetricsSamplerConfig,
    last_message_count: u64,
    last_delta_count: u64,
    last_error_count: u64,
    last_snapshot_time: DateTime<Utc>,
    error_counts: HashMap<String, u64>,
}

impl MetricsSampler {
    /// Create a new metrics sampler
    pub fn new(config: MetricsSamplerConfig) -> Self {
        Self {
            config,
            last_message_count: 0,
            last_delta_count: 0,
            last_error_count: 0,
            last_snapshot_time: Utc::now(),
            error_counts: HashMap::new(),
        }
    }

    /// Create metrics sampler with service name for identification
    pub fn new_with_name(config: MetricsSamplerConfig, _service_name: String) -> Self {
        // Service name could be used for tagging/labeling in the future
        Self::new(config)
    }

    /// Sample current metrics and create a snapshot
    pub fn sample_metrics(
        &mut self,
        current_metrics: &crate::metrics::MetricsCollector,
    ) -> MetricsSnapshot {
        let now = Utc::now();
        let time_delta = now
            .signed_duration_since(self.last_snapshot_time)
            .num_seconds() as f64;

        // Calculate rates
        let current_messages = current_metrics.get_total_messages();
        let current_deltas = current_metrics.get_total_deltas();
        let current_errors = current_metrics.get_total_errors();

        let messages_per_second = if time_delta > 0.0 {
            (current_messages.saturating_sub(self.last_message_count)) as f64 / time_delta
        } else {
            0.0
        };

        let deltas_per_second = if time_delta > 0.0 {
            (current_deltas.saturating_sub(self.last_delta_count)) as f64 / time_delta
        } else {
            0.0
        };

        let error_rate = if time_delta > 0.0 {
            (current_errors.saturating_sub(self.last_error_count)) as f64 / time_delta
        } else {
            0.0
        };

        // Get session state breakdown
        let session_states = SessionStateBreakdown {
            active: current_metrics.get_active_sessions(),
        };

        // Get top errors (simplified for now)
        let top_errors = self.get_top_errors();

        // System metrics (simplified for now)
        let (memory_usage_mb, cpu_usage_percent) = if self.config.enable_system_metrics {
            self.get_system_metrics()
        } else {
            (0.0, 0.0)
        };

        let snapshot = MetricsSnapshot {
            timestamp: now,
            active_sessions: session_states.active,
            messages_per_second,
            deltas_per_second,
            p95_latency_ms: current_metrics.get_p95_latency(),
            p99_latency_ms: current_metrics.get_p99_latency(),
            error_rate,
            memory_usage_mb,
            cpu_usage_percent,
            session_states,
            top_errors,
        };

        // Update state for next sample
        self.last_message_count = current_messages;
        self.last_delta_count = current_deltas;
        self.last_error_count = current_errors;
        self.last_snapshot_time = now;

        snapshot
    }

    /// Get top error types
    fn get_top_errors(&self) -> Vec<ErrorCount> {
        let mut errors: Vec<_> = self
            .error_counts
            .iter()
            .map(|(error_type, count)| ErrorCount {
                error_type: error_type.clone(),
                count: *count,
            })
            .collect();

        errors.sort_by(|a, b| b.count.cmp(&a.count));
        errors.truncate(self.config.max_error_types);
        errors
    }

    /// Get system metrics (simplified implementation)
    fn get_system_metrics(&self) -> (f64, f64) {
        // In a real implementation, this would use system APIs
        // For now, return placeholder values
        (0.0, 0.0)
    }

    /// Record an error for tracking
    pub fn record_error(&mut self, error_type: &str) {
        *self.error_counts.entry(error_type.to_string()).or_insert(0) += 1;
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_snapshot_size() {
        let mut snapshot = MetricsSnapshot::new();

        // Fill with realistic data
        snapshot.active_sessions = 100;
        snapshot.messages_per_second = 150.5;
        snapshot.deltas_per_second = 75.2;
        snapshot.p95_latency_ms = 25.3;
        snapshot.p99_latency_ms = 45.7;
        snapshot.error_rate = 0.5;
        snapshot.memory_usage_mb = 256.8;
        snapshot.cpu_usage_percent = 15.2;

        // Add some error data
        snapshot.top_errors = vec![
            ErrorCount {
                error_type: "connection_timeout".to_string(),
                count: 5,
            },
            ErrorCount {
                error_type: "parse_error".to_string(),
                count: 3,
            },
        ];

        // Verify size is within limits
        let size = snapshot.estimated_json_size();
        println!("MetricsSnapshot JSON size: {size} bytes");
        assert!(size <= 1024, "Snapshot size {size} exceeds 1KB limit");

        // Validation should pass
        assert!(snapshot.validate_size().is_ok());
    }

    #[test]
    fn test_metrics_snapshot_json_serialization() {
        let snapshot = MetricsSnapshot::new();

        // Should serialize and deserialize without errors
        let json = serde_json::to_string(&snapshot).unwrap();
        let deserialized: MetricsSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(snapshot.active_sessions, deserialized.active_sessions);
        assert_eq!(
            snapshot.messages_per_second,
            deserialized.messages_per_second
        );
    }

    #[test]
    fn test_metrics_sampler() {
        let config = MetricsSamplerConfig::default();
        let mut sampler = MetricsSampler::new(config);

        // Mock metrics collector
        let metrics = crate::metrics::MetricsCollector::new();

        let snapshot = sampler.sample_metrics(&metrics);

        // Should create valid snapshot
        assert!(snapshot.validate_size().is_ok());
        assert_eq!(snapshot.active_sessions, 0); // No sessions initially
    }

    #[test]
    fn test_error_tracking() {
        let config = MetricsSamplerConfig {
            max_error_types: 2,
            ..Default::default()
        };
        let mut sampler = MetricsSampler::new(config);

        // Record some errors
        sampler.record_error("timeout");
        sampler.record_error("timeout");
        sampler.record_error("parse_error");
        sampler.record_error("network_error");

        let top_errors = sampler.get_top_errors();

        // Should have max 2 error types, sorted by count
        assert!(top_errors.len() <= 2);
        if !top_errors.is_empty() {
            assert_eq!(top_errors[0].error_type, "timeout");
            assert_eq!(top_errors[0].count, 2);
        }
    }

    #[test]
    fn test_sampler_config_defaults() {
        let config = MetricsSamplerConfig::default();

        assert_eq!(config.sample_interval_seconds, 1);
        assert_eq!(config.max_error_types, 5);
        assert!(config.enable_system_metrics);
    }
}
