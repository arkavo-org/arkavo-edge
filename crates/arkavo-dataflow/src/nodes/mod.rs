pub mod auth_manager;
#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::collection_is_never_read)]
#[allow(clippy::needless_collect)]
mod auth_manager_test;
#[cfg(any(feature = "llm-remote", feature = "llm-local"))]
pub mod llm;
#[cfg(any(feature = "llm-remote", feature = "llm-local"))]
pub mod llm_config;
#[cfg(any(feature = "llm-remote", feature = "llm-local"))]
pub mod llm_discovery;
#[cfg(any(feature = "llm-remote", feature = "llm-local"))]
pub mod model_registry;
pub mod sink;
pub mod source;
pub mod transform;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

#[async_trait]
pub trait NodeProcessor: Send + Sync {
    async fn process(
        &self,
        input: Option<Value>,
        params: &HashMap<String, Value>,
    ) -> Result<Option<Value>>;
    fn node_type(&self) -> &'static str;
}

pub struct NodeRegistry {
    processors: HashMap<String, Box<dyn NodeProcessor>>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            processors: HashMap::new(),
        };

        // Register built-in node types
        registry.register_defaults();

        registry
    }

    fn register_defaults(&mut self) {
        // Sources
        self.register("timer_source", Box::new(source::TimerSource));
        self.register("webhook_source", Box::new(source::WebhookSource));

        // Transforms
        self.register("json_transform", Box::new(transform::JsonTransform));
        self.register("filter_transform", Box::new(transform::FilterTransform));
        #[cfg(any(feature = "llm-remote", feature = "llm-local"))]
        self.register("llm_transform", Box::new(llm::LlmTransform::new()));

        // Sinks
        self.register("console_sink", Box::new(sink::ConsoleSink));
        self.register("file_sink", Box::new(sink::FileSink));
    }

    pub fn register(&mut self, name: &str, processor: Box<dyn NodeProcessor>) {
        self.processors.insert(name.to_string(), processor);
    }

    pub fn get(&self, name: &str) -> Option<&dyn NodeProcessor> {
        self.processors.get(name).map(std::convert::AsRef::as_ref)
    }

    pub fn list(&self) -> Vec<&str> {
        self.processors
            .keys()
            .map(std::string::String::as_str)
            .collect()
    }
}

impl Default for NodeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::collection_is_never_read)]
#[allow(clippy::needless_collect)]
mod tests {
    use super::*;

    #[test]
    fn test_node_registry() {
        let registry = NodeRegistry::new();

        // Check that default nodes are registered
        assert!(registry.get("timer_source").is_some());
        assert!(registry.get("json_transform").is_some());
        assert!(registry.get("console_sink").is_some());

        // Check listing
        let nodes = registry.list();
        assert!(nodes.contains(&"timer_source"));
        assert!(nodes.contains(&"json_transform"));
        assert!(nodes.contains(&"console_sink"));
    }
}
