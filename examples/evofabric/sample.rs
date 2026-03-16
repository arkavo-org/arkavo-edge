use std::collections::HashMap;

/// Configuration for a processing pipeline.
pub struct PipelineConfig {
    pub name: String,
    pub max_retries: u32,
    pub timeout_ms: u64,
    pub tags: HashMap<String, String>,
}

impl PipelineConfig {
    pub fn new(name: String) -> Self {
        Self {
            name,
            max_retries: 3,
            timeout_ms: 5000,
            tags: HashMap::new(),
        }
    }

    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    pub fn is_valid(&self) -> bool {
        !self.name.is_empty() && self.timeout_ms > 0
    }
}

/// Check if a model name indicates a sub-1B parameter model.
/// Sub-1B models lack capacity for useful chain-of-thought reasoning.
fn is_small_model(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("0.6b")
        || lower.contains("0.8b")
        || lower.contains("270m")
        || lower.contains("500m")
}

fn process(cfg: &PipelineConfig) -> bool {
    cfg.is_valid() && cfg.max_retries > 0
}
