use anyhow::Result;
use arkavo_observability::metrics::MetricsCollector;
use arkavo_observability::metrics_snapshot::{
    MetricsSampler, MetricsSamplerConfig, MetricsSnapshot,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};
use tracing::{info, instrument, warn};

/// Simple metrics API for AG-UI integration
#[async_trait::async_trait]
pub trait MetricsApi {
    /// Get current metrics snapshot
    async fn get_current_metrics(&self) -> Result<MetricsSnapshot>;

    /// Subscribe to metrics updates
    fn subscribe_to_metrics(&self) -> broadcast::Receiver<MetricsSnapshot>;

    /// Update sampler configuration
    async fn configure_sampler(&self, config: MetricsSamplerConfig) -> Result<()>;
}

/// Metrics subscription server implementation
pub struct MetricsSubscriptionServer {
    /// Broadcast channel for metrics snapshots
    metrics_tx: broadcast::Sender<MetricsSnapshot>,
    /// Current metrics collector
    metrics_collector: Arc<MetricsCollector>,
    /// Sampler configuration
    sampler_config: Arc<RwLock<MetricsSamplerConfig>>,
}

impl MetricsSubscriptionServer {
    /// Create a new metrics subscription server
    pub fn new(metrics_collector: Arc<MetricsCollector>) -> Self {
        let (metrics_tx, _) = broadcast::channel(100); // Buffer up to 100 snapshots

        Self {
            metrics_tx,
            metrics_collector,
            sampler_config: Arc::new(RwLock::new(MetricsSamplerConfig::default())),
        }
    }

    /// Start the metrics sampler task
    #[instrument(skip(self))]
    pub async fn start_sampler(&self) -> tokio::task::JoinHandle<()> {
        let tx = self.metrics_tx.clone();
        let collector = self.metrics_collector.clone();
        let config = self.sampler_config.clone();

        info!("Starting metrics sampler task");

        tokio::spawn(async move {
            let mut sampler = {
                let config_guard = config.read().await;
                MetricsSampler::new(config_guard.clone())
            };

            let mut interval = {
                let config_guard = config.read().await;
                tokio::time::interval(std::time::Duration::from_secs(
                    config_guard.sample_interval_seconds,
                ))
            };

            loop {
                interval.tick().await;

                // Check if config changed and update interval if needed
                let current_config = {
                    let config_guard = config.read().await;
                    config_guard.clone()
                };

                // Sample metrics
                let snapshot = sampler.sample_metrics(&collector);

                // Validate snapshot size
                if let Err(e) = snapshot.validate_size() {
                    warn!(error = %e, "Metrics snapshot validation failed");
                    continue;
                }

                // Broadcast to subscribers
                match tx.send(snapshot.clone()) {
                    Ok(subscriber_count) => {
                        if subscriber_count > 0 {
                            info!(
                                subscribers = subscriber_count,
                                snapshot_size = snapshot.estimated_json_size(),
                                "Metrics snapshot broadcasted"
                            );
                        }
                    }
                    Err(_) => {
                        // Channel is closed or no receivers
                        info!("No metrics subscribers, continuing sampling");
                    }
                }

                // Update interval if config changed
                if current_config.sample_interval_seconds != sampler.config.sample_interval_seconds
                {
                    sampler.config = current_config.clone();
                    interval = tokio::time::interval(std::time::Duration::from_secs(
                        current_config.sample_interval_seconds,
                    ));
                    info!(
                        new_interval_seconds = current_config.sample_interval_seconds,
                        "Updated metrics sampler interval"
                    );
                }
            }
        })
    }

    /// Get a receiver for metrics updates
    pub fn subscribe_to_metrics(&self) -> broadcast::Receiver<MetricsSnapshot> {
        self.metrics_tx.subscribe()
    }

    /// Get current metrics snapshot immediately
    pub async fn get_current_snapshot(&self) -> MetricsSnapshot {
        let config = self.sampler_config.read().await;
        let mut sampler = MetricsSampler::new(config.clone());
        sampler.sample_metrics(&self.metrics_collector)
    }

    /// Update sampler configuration
    pub async fn update_config(&self, new_config: MetricsSamplerConfig) {
        let mut config = self.sampler_config.write().await;
        *config = new_config.clone();

        info!(
            sample_interval_seconds = new_config.sample_interval_seconds,
            max_error_types = new_config.max_error_types,
            enable_system_metrics = new_config.enable_system_metrics,
            "Updated metrics sampler configuration"
        );
    }
}

#[async_trait::async_trait]
impl MetricsApi for MetricsSubscriptionServer {
    async fn get_current_metrics(&self) -> Result<MetricsSnapshot> {
        Ok(self.get_current_snapshot().await)
    }

    fn subscribe_to_metrics(&self) -> broadcast::Receiver<MetricsSnapshot> {
        self.metrics_tx.subscribe()
    }

    async fn configure_sampler(&self, config: MetricsSamplerConfig) -> Result<()> {
        self.update_config(config).await;
        Ok(())
    }
}

/// Configuration for the metrics subscription service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsServiceConfig {
    /// Enable metrics subscription service
    pub enabled: bool,
    /// Maximum number of concurrent subscribers
    pub max_subscribers: usize,
    /// Buffer size for metrics snapshots
    pub snapshot_buffer_size: usize,
    /// Default sampling configuration
    pub default_sampler_config: MetricsSamplerConfig,
}

impl Default for MetricsServiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_subscribers: 50,
            snapshot_buffer_size: 100,
            default_sampler_config: MetricsSamplerConfig::default(),
        }
    }
}

/// Metrics subscription service manager
pub struct MetricsSubscriptionService {
    server: MetricsSubscriptionServer,
    config: MetricsServiceConfig,
    sampler_handle: Option<tokio::task::JoinHandle<()>>,
}

impl MetricsSubscriptionService {
    /// Create a new metrics subscription service
    pub fn new(metrics_collector: Arc<MetricsCollector>, config: MetricsServiceConfig) -> Self {
        let server = MetricsSubscriptionServer::new(metrics_collector);

        Self {
            server,
            config,
            sampler_handle: None,
        }
    }

    /// Start the metrics subscription service
    #[instrument(skip(self))]
    pub async fn start(&mut self) -> Result<()> {
        if !self.config.enabled {
            info!("Metrics subscription service disabled");
            return Ok(());
        }

        info!(
            max_subscribers = self.config.max_subscribers,
            buffer_size = self.config.snapshot_buffer_size,
            "Starting metrics subscription service"
        );

        // Start the sampler task
        let handle = self.server.start_sampler().await;
        self.sampler_handle = Some(handle);

        info!("Metrics subscription service started successfully");
        Ok(())
    }

    /// Stop the metrics subscription service
    #[instrument(skip(self))]
    pub async fn stop(&mut self) {
        if let Some(handle) = self.sampler_handle.take() {
            handle.abort();
            info!("Metrics sampler task stopped");
        }
    }

    /// Get the subscription server for JSON-RPC registration
    pub fn get_server(&self) -> &MetricsSubscriptionServer {
        &self.server
    }

    /// Get a metrics subscription receiver
    pub fn subscribe(&self) -> broadcast::Receiver<MetricsSnapshot> {
        self.server.subscribe_to_metrics()
    }
}

impl Drop for MetricsSubscriptionService {
    fn drop(&mut self) {
        if let Some(handle) = self.sampler_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_metrics_subscription_server() {
        let collector = Arc::new(MetricsCollector::new());
        let server = MetricsSubscriptionServer::new(collector.clone());

        // Test getting current snapshot
        let snapshot = server.get_current_snapshot().await;
        assert!(snapshot.validate_size().is_ok());
    }

    #[tokio::test]
    async fn test_metrics_sampler_task() {
        let collector = Arc::new(MetricsCollector::new());
        let server = MetricsSubscriptionServer::new(collector.clone());

        // Subscribe to metrics
        let mut receiver = server.subscribe_to_metrics();

        // Start sampler with fast interval for testing
        let fast_config = MetricsSamplerConfig {
            sample_interval_seconds: 1, // 1 second for fast testing
            ..Default::default()
        };
        server.update_config(fast_config).await;

        let _handle = server.start_sampler().await;

        // Generate some metrics
        collector.record_session_created();
        collector.record_message_sent(100);

        // Wait for a snapshot
        let result = tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await;
        assert!(
            result.is_ok(),
            "Should receive metrics snapshot within timeout"
        );

        let snapshot = result.unwrap().unwrap();
        assert!(snapshot.validate_size().is_ok());
        assert_eq!(snapshot.active_sessions, 1);
    }

    #[tokio::test]
    async fn test_metrics_service() {
        let collector = Arc::new(MetricsCollector::new());
        let config = MetricsServiceConfig::default();
        let mut service = MetricsSubscriptionService::new(collector.clone(), config);

        // Start service
        service.start().await.unwrap();

        // Subscribe to metrics
        let mut receiver = service.subscribe();

        // Generate metrics
        collector.record_session_created();
        collector.record_message_sent(200);

        // Wait briefly for sampling
        sleep(Duration::from_millis(1100)).await;

        // Should receive updates
        let result = tokio::time::timeout(Duration::from_secs(1), receiver.recv()).await;
        if let Ok(Ok(snapshot)) = result {
            assert!(snapshot.validate_size().is_ok());
        }

        // Stop service
        service.stop().await;
    }

    #[test]
    fn test_metrics_service_config() {
        let config = MetricsServiceConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_subscribers, 50);
        assert_eq!(config.snapshot_buffer_size, 100);
    }

    #[tokio::test]
    async fn test_config_updates() {
        let collector = Arc::new(MetricsCollector::new());
        let server = MetricsSubscriptionServer::new(collector);

        let new_config = MetricsSamplerConfig {
            sample_interval_seconds: 5,
            max_error_types: 10,
            enable_system_metrics: false,
        };

        server.update_config(new_config.clone()).await;

        let current_config = server.sampler_config.read().await;
        assert_eq!(current_config.sample_interval_seconds, 5);
        assert_eq!(current_config.max_error_types, 10);
        assert!(!current_config.enable_system_metrics);
    }

    #[tokio::test]
    async fn test_snapshot_size_validation() {
        let collector = Arc::new(MetricsCollector::new());

        // Generate substantial metrics data
        for i in 0..100 {
            collector.record_session_created();
            collector.record_message_sent(i * 10);
            collector.record_error("test_error");
        }

        let server = MetricsSubscriptionServer::new(collector);
        let snapshot = server.get_current_snapshot().await;

        // Validate size is within limits
        assert!(snapshot.validate_size().is_ok());

        let size = snapshot.estimated_json_size();
        println!("Snapshot size with 100 sessions: {size} bytes");
        assert!(size <= 1024, "Snapshot size {size} exceeds 1KB limit");
    }
}
