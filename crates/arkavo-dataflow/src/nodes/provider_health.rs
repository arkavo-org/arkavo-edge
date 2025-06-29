use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::interval;

/// Health status of a provider
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

/// Health check result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckResult {
    pub provider_name: String,
    pub status: HealthStatus,
    pub latency_ms: Option<u64>,
    pub available_models: Vec<String>,
    pub error_message: Option<String>,
    pub checked_at: DateTime<Utc>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Provider health metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMetrics {
    pub provider_name: String,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub average_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub last_success: Option<DateTime<Utc>>,
    pub last_failure: Option<DateTime<Utc>>,
    pub consecutive_failures: u32,
    pub uptime_percentage: f64,
}

impl Default for ProviderMetrics {
    fn default() -> Self {
        Self {
            provider_name: String::new(),
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            average_latency_ms: 0.0,
            p95_latency_ms: 0.0,
            p99_latency_ms: 0.0,
            last_success: None,
            last_failure: None,
            consecutive_failures: 0,
            uptime_percentage: 100.0,
        }
    }
}

/// Provider health monitor
pub struct ProviderHealthMonitor {
    health_status: Arc<RwLock<HashMap<String, HealthCheckResult>>>,
    metrics: Arc<RwLock<HashMap<String, ProviderMetrics>>>,
    latency_history: Arc<RwLock<HashMap<String, Vec<f64>>>>,
    check_interval_secs: u64,
}

impl ProviderHealthMonitor {
    /// Create a new health monitor
    pub fn new(check_interval_secs: u64) -> Self {
        Self {
            health_status: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(RwLock::new(HashMap::new())),
            latency_history: Arc::new(RwLock::new(HashMap::new())),
            check_interval_secs,
        }
    }

    /// Start monitoring providers
    pub fn start_monitoring(&self, providers: Vec<(String, String)>) {
        let health_status = self.health_status.clone();
        let metrics = self.metrics.clone();
        let interval_secs = self.check_interval_secs;

        tokio::spawn(async move {
            let mut interval = interval(std::time::Duration::from_secs(interval_secs));

            loop {
                interval.tick().await;

                for (provider_name, base_url) in &providers {
                    let result = Self::check_provider_health(provider_name, base_url);

                    // Update health status
                    {
                        let mut status = health_status.write().await;
                        status.insert(provider_name.clone(), result.clone());
                    }

                    // Update metrics
                    {
                        let mut provider_metrics = metrics.write().await;
                        let metrics = provider_metrics
                            .entry(provider_name.clone())
                            .or_insert_with(|| ProviderMetrics {
                                provider_name: provider_name.clone(),
                                ..Default::default()
                            });

                        if result.status == HealthStatus::Healthy {
                            metrics.last_success = Some(result.checked_at);
                            metrics.consecutive_failures = 0;
                        } else {
                            metrics.last_failure = Some(result.checked_at);
                            metrics.consecutive_failures += 1;
                        }
                        drop(provider_metrics);
                    }
                }
            }
        });
    }

    /// Get current health status for a provider
    pub async fn get_health_status(&self, provider_name: &str) -> Option<HealthCheckResult> {
        let status = self.health_status.read().await;
        status.get(provider_name).cloned()
    }

    /// Get all provider health statuses
    pub async fn get_all_health_statuses(&self) -> HashMap<String, HealthCheckResult> {
        let status = self.health_status.read().await;
        status.clone()
    }

    /// Get provider metrics
    pub async fn get_provider_metrics(&self, provider_name: &str) -> Option<ProviderMetrics> {
        let metrics = self.metrics.read().await;
        metrics.get(provider_name).cloned()
    }

    /// Record a request result
    ///
    /// # Panics
    ///
    /// Panics if unable to sort latency values for percentile calculation
    #[allow(clippy::significant_drop_tightening)]
    pub async fn record_request(
        &self,
        provider_name: &str,
        success: bool,
        latency_ms: Option<u64>,
    ) {
        let mut metrics = self.metrics.write().await;
        let provider_metrics =
            metrics
                .entry(provider_name.to_string())
                .or_insert_with(|| ProviderMetrics {
                    provider_name: provider_name.to_string(),
                    ..Default::default()
                });

        provider_metrics.total_requests += 1;

        if success {
            provider_metrics.successful_requests += 1;
            provider_metrics.last_success = Some(Utc::now());
            provider_metrics.consecutive_failures = 0;
        } else {
            provider_metrics.failed_requests += 1;
            provider_metrics.last_failure = Some(Utc::now());
            provider_metrics.consecutive_failures += 1;
        }

        // Update latency metrics
        if let Some(latency) = latency_ms {
            let mut history = self.latency_history.write().await;
            let latencies = history
                .entry(provider_name.to_string())
                .or_insert_with(Vec::new);

            latencies.push(latency as f64);

            // Keep only last 1000 samples
            if latencies.len() > 1000 {
                latencies.remove(0);
            }

            // Calculate metrics
            if !latencies.is_empty() {
                provider_metrics.average_latency_ms =
                    latencies.iter().sum::<f64>() / latencies.len() as f64;

                // Calculate percentiles
                let mut sorted_latencies = latencies.clone();
                sorted_latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());

                let p95_index = (latencies.len() as f64 * 0.95) as usize;
                let p99_index = (latencies.len() as f64 * 0.99) as usize;

                provider_metrics.p95_latency_ms =
                    sorted_latencies[p95_index.min(latencies.len() - 1)];
                provider_metrics.p99_latency_ms =
                    sorted_latencies[p99_index.min(latencies.len() - 1)];
            }
        }

        // Update uptime percentage
        if provider_metrics.total_requests > 0 {
            provider_metrics.uptime_percentage = (provider_metrics.successful_requests as f64
                / provider_metrics.total_requests as f64)
                * 100.0;
        }
    }

    /// Check if a provider is healthy enough for use
    pub async fn is_provider_healthy(&self, provider_name: &str) -> bool {
        if let Some(health) = self.get_health_status(provider_name).await {
            matches!(
                health.status,
                HealthStatus::Healthy | HealthStatus::Degraded
            )
        } else {
            false
        }
    }

    /// Get the healthiest provider from a list
    ///
    /// # Panics
    ///
    /// Panics if best_provider unwrap fails during comparison (which should not happen)
    #[allow(clippy::significant_drop_tightening)]
    pub async fn get_healthiest_provider(&self, providers: &[String]) -> Option<String> {
        let statuses = self.get_all_health_statuses().await;
        let metrics = self.metrics.read().await;

        let mut best_provider: Option<(String, f64)> = None;

        for provider_name in providers {
            if let Some(health) = statuses.get(provider_name) {
                if matches!(
                    health.status,
                    HealthStatus::Healthy | HealthStatus::Degraded
                ) {
                    // Calculate provider score based on uptime and latency
                    let provider_metrics = metrics.get(provider_name);
                    let uptime = provider_metrics
                        .map(|m| m.uptime_percentage)
                        .unwrap_or(50.0);
                    let avg_latency = provider_metrics
                        .map(|m| m.average_latency_ms)
                        .unwrap_or(1000.0);

                    // Score: higher uptime is better, lower latency is better
                    let score = uptime / (1.0 + avg_latency / 100.0);

                    if best_provider.is_none() || score > best_provider.as_ref().unwrap().1 {
                        best_provider = Some((provider_name.clone(), score));
                    }
                }
            }
        }

        best_provider.map(|(name, _)| name)
    }

    /// Check provider health
    fn check_provider_health(provider_name: &str, base_url: &str) -> HealthCheckResult {
        let start_time = std::time::Instant::now();

        // Simulate health check - in production, this would make actual API calls
        let (status, available_models, error_message) = if base_url.contains("localhost") {
            // Simulate healthy local provider
            (
                HealthStatus::Healthy,
                vec!["llama3.2:latest".to_string()],
                None,
            )
        } else {
            // Simulate checking remote provider
            if std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                % 10
                < 8
            {
                // 80% of the time it's healthy
                (
                    HealthStatus::Healthy,
                    vec!["llama3.2:latest".to_string()],
                    None,
                )
            } else {
                // 20% of the time it's degraded
                (
                    HealthStatus::Degraded,
                    vec![],
                    Some("High latency detected".to_string()),
                )
            }
        };

        let latency_ms = start_time.elapsed().as_millis() as u64;

        HealthCheckResult {
            provider_name: provider_name.to_string(),
            status,
            latency_ms: Some(latency_ms),
            available_models,
            error_message,
            checked_at: Utc::now(),
            metadata: None,
        }
    }

    /// Export health report
    #[allow(clippy::significant_drop_tightening)]
    pub async fn export_health_report(&self) -> serde_json::Value {
        let statuses = self.health_status.read().await;
        let metrics = self.metrics.read().await;

        serde_json::json!({
            "report_generated_at": Utc::now().to_rfc3339(),
            "health_status": statuses.clone(),
            "metrics": metrics.clone(),
            "summary": {
                "total_providers": statuses.len(),
                "healthy_providers": statuses.values().filter(|h| h.status == HealthStatus::Healthy).count(),
                "degraded_providers": statuses.values().filter(|h| h.status == HealthStatus::Degraded).count(),
                "unhealthy_providers": statuses.values().filter(|h| h.status == HealthStatus::Unhealthy).count(),
            }
        })
    }
}

/// Circuit breaker for provider failover
pub struct CircuitBreaker {
    failure_threshold: u32,
    recovery_timeout: Duration,
    provider_states: Arc<RwLock<HashMap<String, CircuitState>>>,
}

#[derive(Debug, Clone)]
struct CircuitState {
    consecutive_failures: u32,
    last_failure: Option<DateTime<Utc>>,
    is_open: bool,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, recovery_timeout_secs: i64) -> Self {
        Self {
            failure_threshold,
            recovery_timeout: Duration::seconds(recovery_timeout_secs),
            provider_states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if circuit is open (provider should not be used)
    pub async fn is_open(&self, provider_name: &str) -> bool {
        let mut states = self.provider_states.write().await;
        let state = states
            .entry(provider_name.to_string())
            .or_insert(CircuitState {
                consecutive_failures: 0,
                last_failure: None,
                is_open: false,
            });

        // Check if circuit should be closed after recovery timeout
        if state.is_open {
            if let Some(last_failure) = state.last_failure {
                if Utc::now() - last_failure > self.recovery_timeout {
                    state.is_open = false;
                    state.consecutive_failures = 0;
                }
            }
        }

        let is_open = state.is_open;
        drop(states);
        is_open
    }

    /// Record a success
    pub async fn record_success(&self, provider_name: &str) {
        let mut states = self.provider_states.write().await;
        if let Some(state) = states.get_mut(provider_name) {
            state.consecutive_failures = 0;
            state.is_open = false;
        }
    }

    /// Record a failure
    pub async fn record_failure(&self, provider_name: &str) {
        let mut states = self.provider_states.write().await;
        let state = states
            .entry(provider_name.to_string())
            .or_insert(CircuitState {
                consecutive_failures: 0,
                last_failure: None,
                is_open: false,
            });

        state.consecutive_failures += 1;
        state.last_failure = Some(Utc::now());

        if state.consecutive_failures >= self.failure_threshold {
            state.is_open = true;
        }
        drop(states);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_health_monitor() {
        let monitor = ProviderHealthMonitor::new(60);

        // Record some requests
        monitor.record_request("local-ollama", true, Some(50)).await;
        monitor.record_request("local-ollama", true, Some(60)).await;
        monitor.record_request("local-ollama", false, None).await;

        // Check metrics
        let metrics = monitor.get_provider_metrics("local-ollama").await.unwrap();
        assert_eq!(metrics.total_requests, 3);
        assert_eq!(metrics.successful_requests, 2);
        assert_eq!(metrics.failed_requests, 1);
        assert!(metrics.average_latency_ms > 0.0);
    }

    #[tokio::test]
    async fn test_circuit_breaker() {
        let breaker = CircuitBreaker::new(3, 60);

        // Initially closed
        assert!(!breaker.is_open("provider1").await);

        // Record failures
        breaker.record_failure("provider1").await;
        breaker.record_failure("provider1").await;
        assert!(!breaker.is_open("provider1").await); // Still closed

        breaker.record_failure("provider1").await;
        assert!(breaker.is_open("provider1").await); // Now open

        // Success resets the circuit
        breaker.record_success("provider1").await;
        assert!(!breaker.is_open("provider1").await);
    }
}
