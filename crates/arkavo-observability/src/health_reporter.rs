use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    pub component: String,
    pub status: HealthStatus,
    pub message: String,
    pub details: Option<HashMap<String, serde_json::Value>>,
    pub timestamp: DateTime<Utc>,
}

impl HealthReport {
    pub fn healthy(component: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: HealthStatus::Healthy,
            message: message.into(),
            details: None,
            timestamp: Utc::now(),
        }
    }

    pub fn degraded(component: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: HealthStatus::Degraded,
            message: message.into(),
            details: None,
            timestamp: Utc::now(),
        }
    }

    pub fn unhealthy(component: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            status: HealthStatus::Unhealthy,
            message: message.into(),
            details: None,
            timestamp: Utc::now(),
        }
    }

    pub fn with_details(mut self, details: HashMap<String, serde_json::Value>) -> Self {
        self.details = Some(details);
        self
    }
}

#[async_trait]
pub trait HealthReporter: Send + Sync {
    async fn check_health(&self) -> HealthReport;
    fn component_name(&self) -> &str;
}

pub struct HealthRegistry {
    reporters: Arc<RwLock<HashMap<String, Arc<dyn HealthReporter>>>>,
}

impl HealthRegistry {
    pub fn new() -> Self {
        Self {
            reporters: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn global() -> &'static HealthRegistry {
        static INSTANCE: std::sync::OnceLock<HealthRegistry> = std::sync::OnceLock::new();
        INSTANCE.get_or_init(HealthRegistry::new)
    }

    pub async fn register(&self, reporter: Arc<dyn HealthReporter>) {
        let name = reporter.component_name().to_string();
        self.reporters.write().await.insert(name, reporter);
    }

    pub async fn unregister(&self, component_name: &str) {
        self.reporters.write().await.remove(component_name);
    }

    pub async fn check_all(&self) -> Vec<HealthReport> {
        let reporters = self.reporters.read().await;
        let mut reports = Vec::new();

        for reporter in reporters.values() {
            reports.push(reporter.check_health().await);
        }

        reports
    }

    pub async fn check_component(&self, component_name: &str) -> Option<HealthReport> {
        let reporters = self.reporters.read().await;
        if let Some(reporter) = reporters.get(component_name) {
            Some(reporter.check_health().await)
        } else {
            None
        }
    }

    pub async fn get_overall_status(&self) -> HealthStatus {
        let reports = self.check_all().await;

        if reports.is_empty() {
            return HealthStatus::Unknown;
        }

        if reports.iter().any(|r| r.status == HealthStatus::Unhealthy) {
            HealthStatus::Unhealthy
        } else if reports.iter().any(|r| r.status == HealthStatus::Degraded) {
            HealthStatus::Degraded
        } else if reports.iter().all(|r| r.status == HealthStatus::Healthy) {
            HealthStatus::Healthy
        } else {
            HealthStatus::Unknown
        }
    }
}

impl Default for HealthRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestReporter {
        name: String,
        status: HealthStatus,
    }

    #[async_trait]
    impl HealthReporter for TestReporter {
        async fn check_health(&self) -> HealthReport {
            HealthReport {
                component: self.name.clone(),
                status: self.status.clone(),
                message: "Test report".to_string(),
                details: None,
                timestamp: Utc::now(),
            }
        }

        fn component_name(&self) -> &str {
            &self.name
        }
    }

    #[tokio::test]
    async fn test_health_report_builders() {
        let healthy = HealthReport::healthy("test", "All good");
        assert_eq!(healthy.status, HealthStatus::Healthy);

        let degraded = HealthReport::degraded("test", "Slow");
        assert_eq!(degraded.status, HealthStatus::Degraded);

        let unhealthy = HealthReport::unhealthy("test", "Failed");
        assert_eq!(unhealthy.status, HealthStatus::Unhealthy);
    }

    #[tokio::test]
    async fn test_registry() {
        let registry = HealthRegistry::new();

        let reporter1 = Arc::new(TestReporter {
            name: "test1".to_string(),
            status: HealthStatus::Healthy,
        });

        let reporter2 = Arc::new(TestReporter {
            name: "test2".to_string(),
            status: HealthStatus::Degraded,
        });

        registry.register(reporter1).await;
        registry.register(reporter2).await;

        let reports = registry.check_all().await;
        assert_eq!(reports.len(), 2);

        let overall = registry.get_overall_status().await;
        assert_eq!(overall, HealthStatus::Degraded);
    }
}
