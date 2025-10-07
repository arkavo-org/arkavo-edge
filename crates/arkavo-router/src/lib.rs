#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unused_async)]

pub mod classifier;
pub mod connectivity;
pub mod decision;
pub mod error;
pub mod metrics;
pub mod selector;

pub use classifier::{TaskCategory, TaskClassifier};
pub use connectivity::ConnectivityChecker;
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
    connectivity: Arc<ConnectivityChecker>,
    offline_mode: bool,
}

impl Router {
    pub async fn new() -> Result<Self> {
        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector: Arc::new(ModelSelector::new()),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            connectivity: Arc::new(ConnectivityChecker::new()),
            offline_mode: false,
        })
    }

    pub async fn new_offline() -> Result<Self> {
        Ok(Self {
            classifier: Arc::new(TaskClassifier::new().await?),
            selector: Arc::new(ModelSelector::new()),
            metrics: Arc::new(RwLock::new(RoutingMetrics::new())),
            connectivity: Arc::new(ConnectivityChecker::new()),
            offline_mode: true,
        })
    }

    pub fn set_offline_mode(&mut self, offline: bool) {
        self.offline_mode = offline;
    }

    pub async fn check_connectivity(&self) -> bool {
        self.connectivity.is_online().await
    }

    pub async fn route(&self, task_description: &str) -> Result<RoutingDecision> {
        let classification = self.classifier.classify(task_description).await?;

        let mut decision = self.selector.select(&classification, task_description)?;

        if self.offline_mode || !self.connectivity.is_online().await {
            if decision.recommended_model.is_cloud() {
                let local_model = self.get_local_fallback(&classification.category);

                decision.reasoning = format!(
                    "Offline mode: Using local {}. Original: {}",
                    local_model.name(),
                    decision.reasoning
                );

                decision.recommended_model = local_model;
                decision.estimated_cost_usd = 0.0;
                decision.should_compress = false;
            }
        }

        self.metrics
            .write()
            .await
            .record_routing(&classification, &decision);

        Ok(decision)
    }

    fn get_local_fallback(&self, category: &TaskCategory) -> ModelChoice {
        match category {
            TaskCategory::FrontendUI | TaskCategory::BackendAPI | TaskCategory::Refactoring => {
                ModelChoice::LocalGemma4B
            }
            _ => ModelChoice::LocalGemma4B,
        }
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
