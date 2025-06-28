pub mod dsl;
pub mod engine;
pub mod nl;
pub mod nodes;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataflowConfig {
    pub max_queue_size: usize,
    pub enable_metrics: bool,
    pub sandbox_enabled: bool,
}

impl Default for DataflowConfig {
    fn default() -> Self {
        Self {
            max_queue_size: 1000,
            enable_metrics: true,
            sandbox_enabled: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataflowEngine {
    config: Arc<DataflowConfig>,
    pipelines: dashmap::DashMap<Uuid, engine::Pipeline>,
}

impl DataflowEngine {
    pub fn new(config: DataflowConfig) -> Self {
        Self {
            config: Arc::new(config),
            pipelines: dashmap::DashMap::new(),
        }
    }

    pub async fn create_pipeline_from_nl(&self, natural_language: &str) -> Result<Uuid> {
        let parser = nl::NLParser::new();
        let blueprint = parser.parse_to_blueprint(natural_language)?;
        self.create_pipeline_from_blueprint(blueprint).await
    }

    pub async fn create_pipeline_from_blueprint(&self, blueprint: dsl::Blueprint) -> Result<Uuid> {
        let pipeline = engine::Pipeline::from_blueprint(blueprint, self.config.clone())?;
        let id = pipeline.id();
        self.pipelines.insert(id, pipeline);
        Ok(id)
    }

    pub fn get_pipeline(&self, id: Uuid) -> Option<engine::Pipeline> {
        self.pipelines.get(&id).map(|p| p.clone())
    }

    pub async fn start_pipeline(&self, id: Uuid) -> Result<()> {
        if let Some(mut pipeline) = self.pipelines.get_mut(&id) {
            pipeline.start().await
        } else {
            Err(anyhow::anyhow!("Pipeline not found"))
        }
    }

    pub async fn stop_pipeline(&self, id: Uuid) -> Result<()> {
        if let Some(mut pipeline) = self.pipelines.get_mut(&id) {
            pipeline.stop().await
        } else {
            Err(anyhow::anyhow!("Pipeline not found"))
        }
    }

    pub fn list_pipelines(&self) -> Vec<(Uuid, String)> {
        self.pipelines
            .iter()
            .map(|entry| (*entry.key(), entry.value().name()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dataflow_config_default() {
        let config = DataflowConfig::default();
        assert_eq!(config.max_queue_size, 1000);
        assert!(config.enable_metrics);
        assert!(config.sandbox_enabled);
    }
}
