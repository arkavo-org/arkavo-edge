//! Lightweight metrics collection for A2A protocol
//!
//! This module provides performance metrics without the overhead of full OpenTelemetry.
//! Metrics are exposed via a Prometheus-compatible endpoint.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

#[cfg(feature = "metrics")]
use metrics::{counter, gauge, histogram};
#[cfg(feature = "metrics")]
use metrics_exporter_prometheus::PrometheusBuilder;

/// Metrics collector for A2A protocol
pub struct MetricsCollector {
    enabled: bool,
}

impl MetricsCollector {
    /// Create a new metrics collector
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Initialize the Prometheus exporter on the given address
    #[cfg(feature = "metrics")]
    pub fn init_prometheus(addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        PrometheusBuilder::new()
            .with_http_listener(addr)
            .install()?;
        Ok(())
    }

    /// Initialize metrics (no-op when metrics feature is disabled)
    #[cfg(not(feature = "metrics"))]
    pub fn init_prometheus(_addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    /// Record an RPC request
    pub fn record_rpc_request(&self, method: &str, success: bool) {
        if !self.enabled {
            return;
        }

        #[cfg(feature = "metrics")]
        {
            let status = if success { "success" } else { "error" };
            counter!("a2a_rpc_total", "method" => method.to_string(), "status" => status.to_string()).increment(1);
        }

        #[cfg(not(feature = "metrics"))]
        {
            let _ = (method, success); // Suppress unused warnings
        }
    }

    /// Record a rate limit block
    pub fn record_rate_limit_blocked(&self, ip: Option<std::net::IpAddr>) {
        if !self.enabled {
            return;
        }

        #[cfg(feature = "metrics")]
        {
            if let Some(ip) = ip {
                counter!("rate_limit_blocked_total", "ip" => ip.to_string()).increment(1);
            } else {
                counter!("rate_limit_blocked_total", "ip" => "global").increment(1);
            }
        }

        #[cfg(not(feature = "metrics"))]
        {
            let _ = ip; // Suppress unused warning
        }
    }

    /// Record mDNS agent discovery
    #[allow(clippy::needless_return)]
    pub fn record_mdns_discovery(&self) {
        if !self.enabled {
            return;
        }

        #[cfg(feature = "metrics")]
        {
            counter!("mdns_agents_discovered_total").increment(1);
        }
    }

    /// Record RPC latency (only on success to avoid hot path overhead)
    pub fn record_rpc_latency(&self, method: &str, duration: std::time::Duration) {
        if !self.enabled {
            return;
        }

        #[cfg(feature = "metrics")]
        {
            histogram!("rpc_latency_seconds", "method" => method.to_string())
                .record(duration.as_secs_f64());
        }

        #[cfg(not(feature = "metrics"))]
        {
            let _ = (method, duration); // Suppress unused warnings
        }
    }

    /// Update rate limiter entry count gauge
    pub fn update_rate_limit_entries(&self, count: usize) {
        if !self.enabled {
            return;
        }

        #[cfg(feature = "metrics")]
        {
            gauge!("rate_limit_entries").set(count as f64);
        }

        #[cfg(not(feature = "metrics"))]
        {
            let _ = count; // Suppress unused warning
        }
    }
}

/// Timer for measuring RPC latency
pub struct RpcTimer {
    method: String,
    start: Instant,
    collector: Arc<MetricsCollector>,
}

impl RpcTimer {
    /// Create a new RPC timer
    pub fn new(method: String, collector: Arc<MetricsCollector>) -> Self {
        Self {
            method,
            start: Instant::now(),
            collector,
        }
    }

    /// Complete the timer and record success
    pub fn success(self) {
        let duration = self.start.elapsed();
        self.collector.record_rpc_request(&self.method, true);
        self.collector.record_rpc_latency(&self.method, duration);
    }

    /// Complete the timer and record error
    pub fn error(self) {
        self.collector.record_rpc_request(&self.method, false);
        // Don't record latency on errors per the plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_collector_creation() {
        let collector = MetricsCollector::new(true);
        assert!(collector.enabled);

        let collector = MetricsCollector::new(false);
        assert!(!collector.enabled);
    }

    #[test]
    fn test_metrics_noop_when_disabled() {
        let collector = MetricsCollector::new(false);

        // These should all be no-ops
        collector.record_rpc_request("test_method", true);
        collector.record_rate_limit_blocked(None);
        collector.record_mdns_discovery();
        collector.update_rate_limit_entries(100);
    }

    #[test]
    fn test_rpc_timer() {
        let collector = Arc::new(MetricsCollector::new(true));
        let timer = RpcTimer::new("test_method".to_string(), collector);

        // Simulate some work
        std::thread::sleep(std::time::Duration::from_millis(10));

        timer.success();
    }
}
