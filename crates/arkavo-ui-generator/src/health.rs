use arkavo_observability::health_reporter::{HealthReport, HealthReporter, HealthStatus};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
struct GenerationMetrics {
    queue_depth: usize,
    total_generations: u64,
    successful_generations: u64,
    failed_generations: u64,
    recent_errors: Vec<GenerationError>,
    average_latency_ms: f64,
    p95_latency_ms: f64,
}

#[derive(Debug, Clone)]
struct GenerationError {
    timestamp: chrono::DateTime<chrono::Utc>,
    error_type: String,
    context: String,
}

impl Default for GenerationMetrics {
    fn default() -> Self {
        Self {
            queue_depth: 0,
            total_generations: 0,
            successful_generations: 0,
            failed_generations: 0,
            recent_errors: Vec::new(),
            average_latency_ms: 0.0,
            p95_latency_ms: 0.0,
        }
    }
}

pub struct UiGeneratorHealthReporter {
    metrics: Arc<RwLock<GenerationMetrics>>,
    latency_history: Arc<RwLock<Vec<f64>>>,
}

impl UiGeneratorHealthReporter {
    pub fn new() -> Self {
        Self {
            metrics: Arc::new(RwLock::new(GenerationMetrics::default())),
            latency_history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn global() -> &'static UiGeneratorHealthReporter {
        static INSTANCE: std::sync::OnceLock<UiGeneratorHealthReporter> =
            std::sync::OnceLock::new();
        INSTANCE.get_or_init(Self::new)
    }

    pub async fn record_generation_start(&self) {
        let mut metrics = self.metrics.write().await;
        metrics.queue_depth += 1;
        metrics.total_generations += 1;
    }

    pub async fn record_generation_complete(&self, latency_ms: u64, success: bool) {
        let mut metrics = self.metrics.write().await;
        metrics.queue_depth = metrics.queue_depth.saturating_sub(1);

        if success {
            metrics.successful_generations += 1;
        } else {
            metrics.failed_generations += 1;
        }

        // Update latency tracking
        let mut history = self.latency_history.write().await;
        history.push(latency_ms as f64);

        // Keep only last 100 samples
        if history.len() > 100 {
            history.remove(0);
        }

        // Calculate metrics
        if !history.is_empty() {
            metrics.average_latency_ms = history.iter().sum::<f64>() / history.len() as f64;

            // Calculate p95
            let mut sorted = history.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let p95_index = ((history.len() as f64 * 0.95).round() as usize).min(history.len() - 1);
            metrics.p95_latency_ms = sorted[p95_index];
        }
    }

    pub async fn record_error(&self, error_type: String, context: String) {
        let mut metrics = self.metrics.write().await;

        metrics.recent_errors.push(GenerationError {
            timestamp: chrono::Utc::now(),
            error_type,
            context,
        });

        // Keep only last 10 errors
        if metrics.recent_errors.len() > 10 {
            metrics.recent_errors.remove(0);
        }
    }

    pub async fn get_queue_depth(&self) -> usize {
        self.metrics.read().await.queue_depth
    }
}

impl Default for UiGeneratorHealthReporter {
    fn default() -> Self {
        Self::new()
    }
}

// Wrapper for registering the global instance with the health registry
pub struct GlobalHealthReporterWrapper;

impl Default for GlobalHealthReporterWrapper {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl HealthReporter for GlobalHealthReporterWrapper {
    async fn check_health(&self) -> HealthReport {
        // Delegate to the global instance
        UiGeneratorHealthReporter::global().check_health().await
    }

    fn component_name(&self) -> &str {
        "ui-generator"
    }
}

#[async_trait]
impl HealthReporter for UiGeneratorHealthReporter {
    async fn check_health(&self) -> HealthReport {
        let metrics = self.metrics.read().await;

        // Determine health status based on metrics
        let status = if !metrics.recent_errors.is_empty()
            && metrics
                .recent_errors
                .iter()
                .any(|e| {
                    e.error_type.contains("API")
                        || e.error_type.contains("auth")
                        || e.error_type.contains("Streaming")
                        || e.context.contains("decoding response body")
                        || e.context.contains("Stream error")
                })
        {
            HealthStatus::Unhealthy
        } else if metrics.queue_depth > 10
            || (metrics.failed_generations > 0
                && metrics.total_generations > 0
                && (metrics.failed_generations as f64 / metrics.total_generations as f64) > 0.2)
            || metrics.average_latency_ms > 10000.0
        {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };

        let message = match status {
            HealthStatus::Healthy => {
                format!(
                    "UI generation working normally. {} generations completed with avg latency {:.0}ms",
                    metrics.successful_generations, metrics.average_latency_ms
                )
            }
            HealthStatus::Degraded => {
                if metrics.queue_depth > 10 {
                    format!(
                        "High queue depth: {} pending generations",
                        metrics.queue_depth
                    )
                } else if metrics.average_latency_ms > 10000.0 {
                    format!(
                        "High latency: avg {:.0}ms, p95 {:.0}ms",
                        metrics.average_latency_ms, metrics.p95_latency_ms
                    )
                } else {
                    format!(
                        "Elevated failure rate: {}/{} generations failed",
                        metrics.failed_generations, metrics.total_generations
                    )
                }
            }
            HealthStatus::Unhealthy => {
                if let Some(error) = metrics.recent_errors.last() {
                    format!("{}: {}", error.error_type, error.context)
                } else {
                    "UI generation system experiencing errors".to_string()
                }
            }
            HealthStatus::Unknown => "UI generation health unknown".to_string(),
        };

        let mut details = HashMap::new();
        details.insert(
            "queue_depth".to_string(),
            serde_json::json!(metrics.queue_depth),
        );
        details.insert(
            "total_generations".to_string(),
            serde_json::json!(metrics.total_generations),
        );
        details.insert(
            "success_rate".to_string(),
            serde_json::json!(if metrics.total_generations > 0 {
                (metrics.successful_generations as f64 / metrics.total_generations as f64) * 100.0
            } else {
                100.0
            }),
        );
        details.insert(
            "average_latency_ms".to_string(),
            serde_json::json!(metrics.average_latency_ms),
        );
        details.insert(
            "p95_latency_ms".to_string(),
            serde_json::json!(metrics.p95_latency_ms),
        );
        details.insert(
            "recent_errors".to_string(),
            serde_json::json!(
                metrics
                    .recent_errors
                    .iter()
                    .map(|e| serde_json::json!({
                        "timestamp": e.timestamp.to_rfc3339(),
                        "type": e.error_type,
                        "context": e.context
                    }))
                    .collect::<Vec<_>>()
            ),
        );

        HealthReport {
            component: "ui-generator".to_string(),
            status,
            message,
            details: Some(details),
            timestamp: chrono::Utc::now(),
        }
    }

    fn component_name(&self) -> &str {
        "ui-generator"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_healthy_state() {
        let reporter = UiGeneratorHealthReporter::new();

        // Record successful generation
        reporter.record_generation_start().await;
        reporter.record_generation_complete(100, true).await;

        let report = reporter.check_health().await;
        assert_eq!(report.status, HealthStatus::Healthy);
        assert_eq!(report.component, "ui-generator");
    }

    #[tokio::test]
    async fn test_degraded_high_queue() {
        let reporter = UiGeneratorHealthReporter::new();

        // Simulate high queue depth
        for _ in 0..15 {
            reporter.record_generation_start().await;
        }

        let report = reporter.check_health().await;
        assert_eq!(report.status, HealthStatus::Degraded);
        assert!(report.message.contains("High queue depth"));
    }

    #[tokio::test]
    async fn test_unhealthy_api_error() {
        let reporter = UiGeneratorHealthReporter::new();

        // Record API error
        reporter
            .record_error(
                "API Error".to_string(),
                "Gemini API key invalid".to_string(),
            )
            .await;

        let report = reporter.check_health().await;
        assert_eq!(report.status, HealthStatus::Unhealthy);
        assert!(report.message.contains("API Error"));
    }
}
