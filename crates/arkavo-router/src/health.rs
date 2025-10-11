use arkavo_observability::health_reporter::{HealthReport, HealthReporter, HealthStatus};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
struct ProviderHealth {
    name: String,
    available: bool,
    last_check: DateTime<Utc>,
    response_time_ms: Option<u64>,
    consecutive_failures: u32,
}

#[derive(Debug, Clone)]
struct RoutingMetrics {
    total_routes: u64,
    cloud_routes: u64,
    local_routes: u64,
    fallback_activations: u64,
    online_mode: bool,
}

impl Default for RoutingMetrics {
    fn default() -> Self {
        Self {
            total_routes: 0,
            cloud_routes: 0,
            local_routes: 0,
            fallback_activations: 0,
            online_mode: true,
        }
    }
}

pub struct RouterHealthReporter {
    providers: Arc<RwLock<HashMap<String, ProviderHealth>>>,
    metrics: Arc<RwLock<RoutingMetrics>>,
    connectivity_checker: Arc<crate::ConnectivityChecker>,
}

impl RouterHealthReporter {
    pub fn new(connectivity_checker: Arc<crate::ConnectivityChecker>) -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
            metrics: Arc::new(RwLock::new(RoutingMetrics::default())),
            connectivity_checker,
        }
    }

    pub async fn record_provider_check(
        &self,
        provider: String,
        available: bool,
        response_time_ms: Option<u64>,
    ) {
        let mut providers = self.providers.write().await;
        let health = providers.entry(provider.clone()).or_insert(ProviderHealth {
            name: provider,
            available: true,
            last_check: Utc::now(),
            response_time_ms: None,
            consecutive_failures: 0,
        });

        health.available = available;
        health.last_check = Utc::now();
        health.response_time_ms = response_time_ms;

        if available {
            health.consecutive_failures = 0;
        } else {
            health.consecutive_failures += 1;
        }
    }

    pub async fn record_routing_decision(&self, used_cloud: bool, was_fallback: bool) {
        let mut metrics = self.metrics.write().await;
        metrics.total_routes += 1;

        if used_cloud {
            metrics.cloud_routes += 1;
        } else {
            metrics.local_routes += 1;
        }

        if was_fallback {
            metrics.fallback_activations += 1;
        }
    }

    pub async fn update_connectivity_status(&self, online: bool) {
        let mut metrics = self.metrics.write().await;
        metrics.online_mode = online;
    }

    async fn get_unhealthy_providers(&self) -> Vec<String> {
        let providers = self.providers.read().await;
        providers
            .values()
            .filter(|p| !p.available || p.consecutive_failures > 2)
            .map(|p| p.name.clone())
            .collect()
    }

    async fn get_degraded_providers(&self) -> Vec<String> {
        let providers = self.providers.read().await;
        providers
            .values()
            .filter(|p| {
                p.available
                    && p.response_time_ms.is_some_and(|ms| ms > 2000)
                    && p.consecutive_failures <= 2
            })
            .map(|p| p.name.clone())
            .collect()
    }
}

impl Default for RouterHealthReporter {
    fn default() -> Self {
        Self::new(Arc::new(crate::ConnectivityChecker::new()))
    }
}

#[async_trait]
impl HealthReporter for RouterHealthReporter {
    async fn check_health(&self) -> HealthReport {
        let providers = self.providers.read().await;
        let metrics = self.metrics.read().await;
        let is_online = self.connectivity_checker.is_online().await;

        let unhealthy_providers = self.get_unhealthy_providers().await;
        let degraded_providers = self.get_degraded_providers().await;

        // Determine overall health status
        let status = if !unhealthy_providers.is_empty() {
            HealthStatus::Unhealthy
        } else if !degraded_providers.is_empty() || !is_online {
            HealthStatus::Degraded
        } else if metrics.fallback_activations > 0
            && metrics.total_routes > 0
            && (metrics.fallback_activations as f64 / metrics.total_routes as f64) > 0.3
        {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };

        let message = match status {
            HealthStatus::Healthy => {
                format!(
                    "Routing healthy. {}/{} requests to cloud providers, {} fallback activations",
                    metrics.cloud_routes, metrics.total_routes, metrics.fallback_activations
                )
            }
            HealthStatus::Degraded => {
                if !is_online {
                    "Running in offline mode - using local models only".to_string()
                } else if !degraded_providers.is_empty() {
                    format!("Degraded providers: {}", degraded_providers.join(", "))
                } else {
                    format!(
                        "High fallback rate: {}/{} requests using fallback",
                        metrics.fallback_activations, metrics.total_routes
                    )
                }
            }
            HealthStatus::Unhealthy => {
                format!("Provider failures: {}", unhealthy_providers.join(", "))
            }
            HealthStatus::Unknown => "Router health unknown".to_string(),
        };

        let mut details = HashMap::new();
        details.insert("online_mode".to_string(), serde_json::json!(is_online));
        details.insert(
            "total_routes".to_string(),
            serde_json::json!(metrics.total_routes),
        );
        details.insert(
            "cloud_routes".to_string(),
            serde_json::json!(metrics.cloud_routes),
        );
        details.insert(
            "local_routes".to_string(),
            serde_json::json!(metrics.local_routes),
        );
        details.insert(
            "fallback_activations".to_string(),
            serde_json::json!(metrics.fallback_activations),
        );
        details.insert(
            "providers".to_string(),
            serde_json::json!(
                providers
                    .values()
                    .map(|p| serde_json::json!({
                        "name": p.name,
                        "available": p.available,
                        "last_check": p.last_check.to_rfc3339(),
                        "response_time_ms": p.response_time_ms,
                        "consecutive_failures": p.consecutive_failures
                    }))
                    .collect::<Vec<_>>()
            ),
        );

        HealthReport {
            component: "router".to_string(),
            status,
            message,
            details: Some(details),
            timestamp: Utc::now(),
        }
    }

    fn component_name(&self) -> &'static str {
        "router"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_healthy_router() {
        let reporter = RouterHealthReporter::default();

        // Record healthy provider
        reporter
            .record_provider_check("gemini".to_string(), true, Some(100))
            .await;

        // Record successful routing
        reporter.record_routing_decision(true, false).await;

        let report = reporter.check_health().await;
        assert_eq!(report.status, HealthStatus::Healthy);
        assert_eq!(report.component, "router");
    }

    #[tokio::test]
    async fn test_degraded_high_fallback() {
        let reporter = RouterHealthReporter::default();

        // Record many fallback activations
        for _ in 0..5 {
            reporter.record_routing_decision(false, true).await;
        }
        for _ in 0..5 {
            reporter.record_routing_decision(true, false).await;
        }

        let report = reporter.check_health().await;
        assert_eq!(report.status, HealthStatus::Degraded);
        assert!(report.message.contains("fallback"));
    }

    #[tokio::test]
    async fn test_unhealthy_provider_failures() {
        let reporter = RouterHealthReporter::default();

        // Record consecutive failures
        for _ in 0..4 {
            reporter
                .record_provider_check("gemini".to_string(), false, None)
                .await;
        }

        let report = reporter.check_health().await;
        assert_eq!(report.status, HealthStatus::Unhealthy);
        assert!(report.message.contains("failures"));
    }
}
