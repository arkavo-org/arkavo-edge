#![deny(clippy::all)]
#![allow(clippy::collapsible_if)]

//! # Arkavo Dataflow
//!
//! A comprehensive dataflow orchestration system that enables natural language-driven
//! data pipeline creation with zero configuration.
//!
//! ## Overview
//!
//! This crate provides a complete dataflow engine with:
//! - Structured DSL with JSON Schema validation
//! - Natural language parser (with `nl` feature)
//! - Security sandbox for safe execution (with `sandbox` feature)
//! - Real-time metrics and performance monitoring
//! - Blueprint version migration system
//!
//! ## Example
//!
//! ```rust,no_run
//! use arkavo_dataflow::{DataflowEngine, DataflowConfig};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create engine with default config
//!     let engine = DataflowEngine::new(DataflowConfig::default());
//!     
//!     // Create pipeline from natural language (requires `nl` feature)
//!     #[cfg(feature = "nl")]
//!     {
//!         let pipeline_id = engine
//!             .create_pipeline_from_nl("Connect input to output when message contains 'error'")?;
//!         
//!         // Start the pipeline
//!         engine.start_pipeline(pipeline_id).await?;
//!     }
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Features
//!
//! - `nl` (default): Enables natural language parsing capabilities
//! - `sandbox` (default): Enables secure code execution sandbox

pub mod agent_interface;
pub mod dsl;
pub mod engine;
pub mod mcp_tools;
#[cfg(feature = "nl")]
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
    #[cfg(feature = "sandbox")]
    pub sandbox_config: Option<engine::sandbox::SandboxConfig>,
}

impl Default for DataflowConfig {
    fn default() -> Self {
        Self {
            max_queue_size: 1000,
            enable_metrics: true,
            sandbox_enabled: true,
            #[cfg(feature = "sandbox")]
            sandbox_config: None, // Uses SandboxConfig::default() when None
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

    #[cfg(feature = "nl")]
    pub fn create_pipeline_from_nl(&self, natural_language: &str) -> Result<Uuid> {
        let parser = nl::NLParser::new();
        let blueprint = parser.parse_to_blueprint(natural_language)?;
        self.create_pipeline_from_blueprint(blueprint)
    }

    pub fn create_pipeline_from_blueprint(&self, blueprint: dsl::Blueprint) -> Result<Uuid> {
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

    pub fn export_blueprint(&self, pipeline_id: Uuid) -> Result<String> {
        let blueprint = self
            .pipelines
            .get(&pipeline_id)
            .ok_or_else(|| anyhow::anyhow!("Pipeline not found"))?
            .to_blueprint();

        let json = serde_json::to_string_pretty(&blueprint)?;
        Ok(json)
    }

    pub fn export_blueprint_compressed(&self, pipeline_id: Uuid) -> Result<Vec<u8>> {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;

        let json = self.export_blueprint(pipeline_id)?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(json.as_bytes())?;
        encoder.finish().map_err(Into::into)
    }

    pub fn import_blueprint(&self, json: &str) -> Result<Uuid> {
        let blueprint: dsl::Blueprint = serde_json::from_str(json)?;
        dsl::BlueprintValidator::validate(&blueprint)
            .map_err(|errors| anyhow::anyhow!("Blueprint validation failed: {:?}", errors))?;
        self.create_pipeline_from_blueprint(blueprint)
    }

    pub fn import_blueprint_compressed(&self, data: &[u8]) -> Result<Uuid> {
        use flate2::read::GzDecoder;
        use std::io::Read;

        let mut decoder = GzDecoder::new(data);
        let mut json = String::new();
        decoder.read_to_string(&mut json)?;
        self.import_blueprint(&json)
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
