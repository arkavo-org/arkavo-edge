use anyhow::Result;
use arkavo_memory::storage::MemoryStorage;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProviderConfig {
    pub name: String,
    pub base_url: String,
    pub default_model: Option<String>,
    pub auth_ref: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfiguration {
    pub providers: Vec<LlmProviderConfig>,
    pub model_preferences: HashMap<String, String>, // task_type -> model
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl LlmConfiguration {
    pub fn new() -> Self {
        Self {
            providers: vec![LlmProviderConfig {
                name: "local-ollama".to_string(),
                base_url: "http://localhost:11434".to_string(),
                default_model: None,
                auth_ref: None,
                description: Some("Local Ollama instance".to_string()),
            }],
            model_preferences: HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    pub fn add_provider(&mut self, provider: LlmProviderConfig) {
        self.providers.push(provider);
        self.updated_at = chrono::Utc::now();
    }

    pub fn get_provider(&self, name: &str) -> Option<&LlmProviderConfig> {
        self.providers.iter().find(|p| p.name == name)
    }

    pub fn set_model_preference(&mut self, task_type: String, model: String) {
        self.model_preferences.insert(task_type, model);
        self.updated_at = chrono::Utc::now();
    }

    pub fn get_model_for_task(&self, task_type: &str) -> Option<&String> {
        self.model_preferences.get(task_type)
    }
}

impl Default for LlmConfiguration {
    fn default() -> Self {
        Self::new()
    }
}

/// Store LLM configuration in memory storage
pub async fn store_llm_config(config: &LlmConfiguration) -> Result<String> {
    let content = serde_json::to_string_pretty(config)?;

    // Create storage instance
    let storage = MemoryStorage::new().await?;

    // Create memory entry
    let memory = arkavo_memory::models::Memory {
        id: uuid::Uuid::new_v4(),
        content: content.clone(),
        embedding: vec![],
        category: Some("llm_config".to_string()),
        metadata: Some(json!({
            "type": "llm_configuration",
            "version": "1.0",
            "provider_count": config.providers.len(),
            "updated_at": config.updated_at
        })),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    // Store in memory
    storage.store(memory).await?;

    Ok(content)
}

/// Retrieve LLM configuration from memory storage
pub async fn load_llm_config() -> Result<Option<LlmConfiguration>> {
    // Create storage instance
    let storage = MemoryStorage::new().await?;

    // Search for latest configuration
    let results = storage.search("", 1, Some("llm_config")).await?;

    if let Some(result) = results.first() {
        let config: LlmConfiguration = serde_json::from_str(&result.memory.content)?;
        return Ok(Some(config));
    }

    Ok(None)
}

/// Interactive configuration builder that prompts for user input
pub struct LlmConfigBuilder {
    config: LlmConfiguration,
}

impl LlmConfigBuilder {
    pub fn new() -> Self {
        Self {
            config: LlmConfiguration::new(),
        }
    }

    pub fn with_existing(config: LlmConfiguration) -> Self {
        Self { config }
    }

    pub fn add_remote_ollama(&mut self, url: &str, name: Option<&str>) -> Result<()> {
        let provider_name = name.unwrap_or("remote-ollama");

        let provider = LlmProviderConfig {
            name: provider_name.to_string(),
            base_url: url.to_string(),
            default_model: None,
            auth_ref: None,
            description: Some(format!("Remote Ollama instance at {url}")),
        };

        self.config.add_provider(provider);
        Ok(())
    }

    pub fn set_task_model(&mut self, task: &str, model: &str) {
        self.config
            .set_model_preference(task.to_string(), model.to_string());
    }

    pub fn build(self) -> LlmConfiguration {
        self.config
    }
}

impl Default for LlmConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_configuration() {
        let mut config = LlmConfiguration::new();

        // Add a remote provider
        config.add_provider(LlmProviderConfig {
            name: "edge-box".to_string(),
            base_url: "http://10.0.0.101:11434".to_string(),
            default_model: Some("devstral:latest".to_string()),
            auth_ref: None,
            description: Some("Edge box Ollama".to_string()),
        });

        // Set model preferences
        config.set_model_preference("code_review".to_string(), "devstral:latest".to_string());
        config.set_model_preference("summarization".to_string(), "llama3.2:latest".to_string());

        assert_eq!(config.providers.len(), 2);
        assert_eq!(
            config.get_model_for_task("code_review"),
            Some(&"devstral:latest".to_string())
        );
    }
}
