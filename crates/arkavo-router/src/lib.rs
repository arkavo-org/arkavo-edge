#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unused_async)]

pub mod classifier;
pub mod decision;
pub mod error;
pub mod metrics;
pub mod selector;

pub use classifier::{TaskCategory, TaskClassifier};
pub use decision::{ModelChoice, RoutingDecision};
pub use error::{Error, Result};
pub use metrics::RoutingMetrics;
pub use selector::ModelSelector;

use std::sync::Arc;
use tokio::sync::RwLock;

/// Intelligent router for cost-optimized model selection
pub struct Router {
    classifier: Arc<TaskClassifier>,
    selector: Arc<ModelSelector>,
    metrics: Arc<RwLock<RoutingMetrics>>,
}

impl Router {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector: Arc::new(ModelSelector::new()),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
        })
    }

    pub async fn route(&self, task_description: &str) -> Result<RoutingDecision> {
        let classification = self.classifier.classify(task_description).await?;

        let decision = self.selector.select(&classification, task_description)?;

        self.metrics
            .write()
            .await
            .record_routing(&classification, &decision);

        Ok(decision)
    }

    pub async fn get_metrics(&self) -> RoutingMetrics {
        self.metrics.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_router_creation() {
        let result = Router::new().await;
        assert!(result.is_ok());
    }
}
