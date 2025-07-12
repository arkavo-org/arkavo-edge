use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info, instrument};

/// Application-wide metrics collector
#[derive(Clone)]
pub struct MetricsCollector {
    // Session metrics
    pub total_sessions_created: Arc<AtomicU64>,
    pub active_sessions: Arc<AtomicU64>,
    pub total_sessions_closed: Arc<AtomicU64>,

    // Message metrics
    pub total_messages_sent: Arc<AtomicU64>,
    pub total_messages_received: Arc<AtomicU64>,
    pub total_deltas_sent: Arc<AtomicU64>,

    // Error metrics
    pub total_errors: Arc<AtomicU64>,
    pub llm_errors: Arc<AtomicU64>,
    pub session_errors: Arc<AtomicU64>,

    // Performance metrics
    pub avg_response_time_ms: Arc<AtomicU64>,
    pub total_requests: Arc<AtomicU64>,

    // System metrics
    pub memory_usage_bytes: Arc<AtomicU64>,
    pub cpu_usage_percent: Arc<AtomicU64>,
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new() -> Self {
        info!("Metrics collector initialized");
        Self {
            total_sessions_created: Arc::new(AtomicU64::new(0)),
            active_sessions: Arc::new(AtomicU64::new(0)),
            total_sessions_closed: Arc::new(AtomicU64::new(0)),
            total_messages_sent: Arc::new(AtomicU64::new(0)),
            total_messages_received: Arc::new(AtomicU64::new(0)),
            total_deltas_sent: Arc::new(AtomicU64::new(0)),
            total_errors: Arc::new(AtomicU64::new(0)),
            llm_errors: Arc::new(AtomicU64::new(0)),
            session_errors: Arc::new(AtomicU64::new(0)),
            avg_response_time_ms: Arc::new(AtomicU64::new(0)),
            total_requests: Arc::new(AtomicU64::new(0)),
            memory_usage_bytes: Arc::new(AtomicU64::new(0)),
            cpu_usage_percent: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Record a new session creation
    #[instrument(skip(self))]
    pub fn record_session_created(&self) {
        let created = self.total_sessions_created.fetch_add(1, Ordering::Relaxed) + 1;
        let active = self.active_sessions.fetch_add(1, Ordering::Relaxed) + 1;

        info!(
            sessions.total_created = created,
            sessions.active = active,
            "Session created recorded"
        );
    }

    /// Record a session closure
    #[instrument(skip(self))]
    pub fn record_session_closed(&self) {
        let closed = self.total_sessions_closed.fetch_add(1, Ordering::Relaxed) + 1;
        let active = self
            .active_sessions
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1);

        info!(
            sessions.total_closed = closed,
            sessions.active = active,
            "Session closed recorded"
        );
    }

    /// Record a message sent
    #[instrument(skip(self))]
    pub fn record_message_sent(&self, length: usize) {
        let total = self.total_messages_sent.fetch_add(1, Ordering::Relaxed) + 1;

        info!(
            messages.total_sent = total,
            message.length = length,
            "Message sent recorded"
        );
    }

    /// Record a message received
    #[instrument(skip(self))]
    pub fn record_message_received(&self) {
        let total = self.total_messages_received.fetch_add(1, Ordering::Relaxed) + 1;

        info!(messages.total_received = total, "Message received recorded");
    }

    /// Record a delta sent
    #[instrument(skip(self))]
    pub fn record_delta_sent(&self, delta_type: &str) {
        let total = self.total_deltas_sent.fetch_add(1, Ordering::Relaxed) + 1;

        info!(
            deltas.total_sent = total,
            delta.type = delta_type,
            "Delta sent recorded"
        );
    }

    /// Record an error
    #[instrument(skip(self))]
    pub fn record_error(&self, error_type: &str) {
        let total = self.total_errors.fetch_add(1, Ordering::Relaxed) + 1;

        match error_type {
            "llm" => {
                self.llm_errors.fetch_add(1, Ordering::Relaxed);
            }
            "session" => {
                self.session_errors.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }

        info!(
            errors.total = total,
            error.type = error_type,
            "Error recorded"
        );
    }

    /// Record response time
    #[instrument(skip(self))]
    pub fn record_response_time(&self, duration_ms: u64) {
        let total_requests = self.total_requests.fetch_add(1, Ordering::Relaxed) + 1;

        // Simple moving average calculation
        let current_avg = self.avg_response_time_ms.load(Ordering::Relaxed);
        let new_avg = if total_requests == 1 {
            duration_ms
        } else {
            (current_avg * (total_requests - 1) + duration_ms) / total_requests
        };

        self.avg_response_time_ms.store(new_avg, Ordering::Relaxed);

        info!(
            response.duration_ms = duration_ms,
            response.avg_ms = new_avg,
            response.total_requests = total_requests,
            "Response time recorded"
        );
    }

    /// Update system metrics
    #[instrument(skip(self))]
    pub fn update_system_metrics(&self, memory_bytes: u64, cpu_percent: u64) {
        self.memory_usage_bytes
            .store(memory_bytes, Ordering::Relaxed);
        self.cpu_usage_percent.store(cpu_percent, Ordering::Relaxed);

        info!(
            system.memory_bytes = memory_bytes,
            system.cpu_percent = cpu_percent,
            "System metrics updated"
        );
    }

    /// Get current metrics snapshot
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            timestamp: Utc::now(),
            total_sessions_created: self.total_sessions_created.load(Ordering::Relaxed),
            active_sessions: self.active_sessions.load(Ordering::Relaxed),
            total_sessions_closed: self.total_sessions_closed.load(Ordering::Relaxed),
            total_messages_sent: self.total_messages_sent.load(Ordering::Relaxed),
            total_messages_received: self.total_messages_received.load(Ordering::Relaxed),
            total_deltas_sent: self.total_deltas_sent.load(Ordering::Relaxed),
            total_errors: self.total_errors.load(Ordering::Relaxed),
            llm_errors: self.llm_errors.load(Ordering::Relaxed),
            session_errors: self.session_errors.load(Ordering::Relaxed),
            avg_response_time_ms: self.avg_response_time_ms.load(Ordering::Relaxed),
            total_requests: self.total_requests.load(Ordering::Relaxed),
            memory_usage_bytes: self.memory_usage_bytes.load(Ordering::Relaxed),
            cpu_usage_percent: self.cpu_usage_percent.load(Ordering::Relaxed),
        }
    }

    /// Get P95 latency in milliseconds (simplified estimation)
    pub fn get_p95_latency(&self) -> f64 {
        let avg = self.avg_response_time_ms.load(Ordering::Relaxed) as f64;
        avg * 1.5 // Estimate P95 as 1.5x average
    }

    /// Get P99 latency in milliseconds (simplified estimation)
    pub fn get_p99_latency(&self) -> f64 {
        let avg = self.avg_response_time_ms.load(Ordering::Relaxed) as f64;
        avg * 2.0 // Estimate P99 as 2x average
    }

    /// Get total message count
    pub fn get_total_messages(&self) -> u64 {
        self.total_messages_sent.load(Ordering::Relaxed)
    }

    /// Get total delta count
    pub fn get_total_deltas(&self) -> u64 {
        self.total_deltas_sent.load(Ordering::Relaxed)
    }

    /// Get total error count
    pub fn get_total_errors(&self) -> u64 {
        self.total_errors.load(Ordering::Relaxed)
    }

    /// Get active sessions count
    pub fn get_active_sessions(&self) -> u64 {
        self.active_sessions.load(Ordering::Relaxed)
    }

    /// Get closing sessions count (placeholder - would integrate with session state tracking)
    pub fn get_closing_sessions(&self) -> u64 {
        // This would integrate with SessionState tracking in a real implementation
        0
    }

    /// Get zombie sessions count (placeholder - would integrate with session state tracking)
    pub fn get_zombie_sessions(&self) -> u64 {
        // This would integrate with SessionState tracking in a real implementation
        0
    }

    /// Reset all metrics (useful for testing)
    #[cfg(test)]
    pub fn reset(&self) {
        use std::sync::atomic::AtomicU64;

        // Create new atomic values to reset everything
        let _zero = AtomicU64::new(0);

        self.total_sessions_created.store(0, Ordering::Relaxed);
        self.active_sessions.store(0, Ordering::Relaxed);
        self.total_sessions_closed.store(0, Ordering::Relaxed);
        self.total_messages_sent.store(0, Ordering::Relaxed);
        self.total_messages_received.store(0, Ordering::Relaxed);
        self.total_deltas_sent.store(0, Ordering::Relaxed);
        self.total_errors.store(0, Ordering::Relaxed);
        self.llm_errors.store(0, Ordering::Relaxed);
        self.session_errors.store(0, Ordering::Relaxed);
        self.avg_response_time_ms.store(0, Ordering::Relaxed);
        self.total_requests.store(0, Ordering::Relaxed);
        self.memory_usage_bytes.store(0, Ordering::Relaxed);
        self.cpu_usage_percent.store(0, Ordering::Relaxed);
    }
}

/// Immutable snapshot of current metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    pub timestamp: DateTime<Utc>,
    pub total_sessions_created: u64,
    pub active_sessions: u64,
    pub total_sessions_closed: u64,
    pub total_messages_sent: u64,
    pub total_messages_received: u64,
    pub total_deltas_sent: u64,
    pub total_errors: u64,
    pub llm_errors: u64,
    pub session_errors: u64,
    pub avg_response_time_ms: u64,
    pub total_requests: u64,
    pub memory_usage_bytes: u64,
    pub cpu_usage_percent: u64,
}

impl MetricsSnapshot {
    /// Calculate derived metrics
    pub fn derived_metrics(&self) -> DerivedMetrics {
        let error_rate = if self.total_requests > 0 {
            (self.total_errors as f64 / self.total_requests as f64) * 100.0
        } else {
            0.0
        };

        let messages_per_session = if self.total_sessions_created > 0 {
            self.total_messages_sent as f64 / self.total_sessions_created as f64
        } else {
            0.0
        };

        DerivedMetrics {
            error_rate_percent: error_rate,
            messages_per_session,
            memory_usage_mb: self.memory_usage_bytes as f64 / (1024.0 * 1024.0),
        }
    }
}

/// Derived metrics calculated from base metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DerivedMetrics {
    pub error_rate_percent: f64,
    pub messages_per_session: f64,
    pub memory_usage_mb: f64,
}

/// Metrics reporter for periodic logging and export
pub struct MetricsReporter {
    collector: MetricsCollector,
    report_interval_seconds: u64,
}

impl MetricsReporter {
    /// Create a new metrics reporter
    pub fn new(collector: MetricsCollector, report_interval_seconds: u64) -> Self {
        Self {
            collector,
            report_interval_seconds,
        }
    }

    /// Start periodic metrics reporting
    pub async fn start_reporting(self) -> tokio::task::JoinHandle<()> {
        info!(
            report_interval_seconds = self.report_interval_seconds,
            "Starting metrics reporter"
        );

        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(self.report_interval_seconds));

            loop {
                interval.tick().await;

                let snapshot = self.collector.snapshot();
                let derived = snapshot.derived_metrics();

                info!(
                    metrics.timestamp = %snapshot.timestamp,
                    metrics.active_sessions = snapshot.active_sessions,
                    metrics.total_sessions = snapshot.total_sessions_created,
                    metrics.total_messages = snapshot.total_messages_sent,
                    metrics.error_rate_percent = derived.error_rate_percent,
                    metrics.avg_response_ms = snapshot.avg_response_time_ms,
                    metrics.memory_mb = derived.memory_usage_mb,
                    metrics.cpu_percent = snapshot.cpu_usage_percent,
                    "Periodic metrics report"
                );
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_collector() {
        let collector = MetricsCollector::new();

        // Test session metrics
        collector.record_session_created();
        assert_eq!(collector.total_sessions_created.load(Ordering::Relaxed), 1);
        assert_eq!(collector.active_sessions.load(Ordering::Relaxed), 1);

        collector.record_session_closed();
        assert_eq!(collector.total_sessions_closed.load(Ordering::Relaxed), 1);
        assert_eq!(collector.active_sessions.load(Ordering::Relaxed), 0);

        // Test message metrics
        collector.record_message_sent(100);
        assert_eq!(collector.total_messages_sent.load(Ordering::Relaxed), 1);

        collector.record_message_received();
        assert_eq!(collector.total_messages_received.load(Ordering::Relaxed), 1);

        // Test error metrics
        collector.record_error("llm");
        assert_eq!(collector.total_errors.load(Ordering::Relaxed), 1);
        assert_eq!(collector.llm_errors.load(Ordering::Relaxed), 1);

        // Test response time
        collector.record_response_time(100);
        assert_eq!(collector.avg_response_time_ms.load(Ordering::Relaxed), 100);
        assert_eq!(collector.total_requests.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_metrics_snapshot() {
        let collector = MetricsCollector::new();
        collector.record_session_created();
        collector.record_message_sent(100);
        collector.record_response_time(50);

        let snapshot = collector.snapshot();
        assert_eq!(snapshot.total_sessions_created, 1);
        assert_eq!(snapshot.total_messages_sent, 1);
        assert_eq!(snapshot.avg_response_time_ms, 50);

        let derived = snapshot.derived_metrics();
        assert_eq!(derived.messages_per_session, 1.0);
    }

    #[test]
    fn test_response_time_averaging() {
        let collector = MetricsCollector::new();

        collector.record_response_time(100);
        assert_eq!(collector.avg_response_time_ms.load(Ordering::Relaxed), 100);

        collector.record_response_time(200);
        assert_eq!(collector.avg_response_time_ms.load(Ordering::Relaxed), 150);

        collector.record_response_time(300);
        assert_eq!(collector.avg_response_time_ms.load(Ordering::Relaxed), 200);
    }

    #[tokio::test]
    async fn test_metrics_reporter() {
        let collector = MetricsCollector::new();
        collector.record_session_created();
        collector.record_message_sent(100);

        let reporter = MetricsReporter::new(collector, 1);
        let handle = reporter.start_reporting().await;

        // Let it run for a bit
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        handle.abort();
    }
}
