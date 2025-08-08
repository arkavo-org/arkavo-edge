/// Model management module for dynamic LLM selection in the terminal UI.
///
/// This module provides functionality to:
/// - Register and manage multiple LLM models from different providers
/// - Route requests to appropriate model endpoints
/// - Parse model selection from user prompts using @model syntax
/// - Track model capabilities and context lengths
///
/// # Example
///
/// User can select a model by prefixing their prompt with @model:
/// ```text
/// @gpt-4 Explain this code
/// @claude-3 Generate a test suite
/// @llama-3 What is rust?
/// ```
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// Default context lengths for various models
const CONTEXT_LENGTH_GPT4: usize = 8192;
const CONTEXT_LENGTH_CLAUDE3: usize = 200_000;
const CONTEXT_LENGTH_LLAMA3: usize = 4096;

/// Model information
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub name: String,
    pub provider: String,
    pub endpoint: Option<String>,
    pub capabilities: Vec<String>,
    pub context_length: usize,
}

/// Manages available models and their routing
pub struct ModelManager {
    models: Arc<RwLock<HashMap<String, ModelInfo>>>,
    default_model: Arc<RwLock<String>>,
}

impl ModelManager {
    pub fn new() -> Self {
        let mut models = HashMap::new();

        // Add default models
        models.insert(
            "gpt-4".to_string(),
            ModelInfo {
                name: "gpt-4".to_string(),
                provider: "openai".to_string(),
                endpoint: None,
                capabilities: vec!["chat".to_string(), "code".to_string()],
                context_length: CONTEXT_LENGTH_GPT4,
            },
        );

        models.insert(
            "claude-3".to_string(),
            ModelInfo {
                name: "claude-3".to_string(),
                provider: "anthropic".to_string(),
                endpoint: None,
                capabilities: vec!["chat".to_string(), "code".to_string(), "vision".to_string()],
                context_length: CONTEXT_LENGTH_CLAUDE3,
            },
        );

        models.insert(
            "llama-3".to_string(),
            ModelInfo {
                name: "llama-3".to_string(),
                provider: "ollama".to_string(),
                endpoint: Some("http://localhost:11434".to_string()),
                capabilities: vec!["chat".to_string()],
                context_length: CONTEXT_LENGTH_LLAMA3,
            },
        );

        Self {
            models: Arc::new(RwLock::new(models)),
            default_model: Arc::new(RwLock::new("gpt-4".to_string())),
        }
    }

    /// Register a new model
    pub async fn register_model(&self, model: ModelInfo) {
        let mut models = self.models.write().await;
        models.insert(model.name.clone(), model);
    }

    /// Get model info
    pub async fn get_model(&self, name: &str) -> Option<ModelInfo> {
        let models = self.models.read().await;
        models.get(name).cloned()
    }

    /// List all available models
    pub async fn list_models(&self) -> Vec<ModelInfo> {
        let models = self.models.read().await;
        models.values().cloned().collect()
    }

    /// Set default model
    pub async fn set_default_model(&self, name: String) {
        let mut default = self.default_model.write().await;
        *default = name;
    }

    /// Get default model
    pub async fn get_default_model(&self) -> String {
        let default = self.default_model.read().await;
        default.clone()
    }

    /// Parse model from prompt (e.g., "@gpt-4 hello")
    pub fn parse_model_from_prompt(prompt: &str) -> Option<(String, String)> {
        if prompt.starts_with('@') {
            let parts: Vec<&str> = prompt.splitn(2, ' ').collect();
            if parts.len() == 2 {
                let model = parts[0].trim_start_matches('@');
                let actual_prompt = parts[1];
                return Some((model.to_string(), actual_prompt.to_string()));
            }
        }
        None
    }
}

impl Default for ModelManager {
    fn default() -> Self {
        Self::new()
    }
}
