use anyhow::Result;
use arkavo_memory::storage::MemoryStorage;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Information about a specific model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model_id: String,
    pub provider_name: String,
    pub display_name: String,
    pub description: Option<String>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub capabilities: ModelCapabilities,
    pub tags: Vec<String>,
    pub version: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    pub pricing: Option<ModelPricing>,
}

/// Model pricing information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub input_cost_per_thousand_cents: u64,
    pub output_cost_per_thousand_cents: u64,
    pub effective_date: DateTime<Utc>,
    pub currency: String,
}

/// Model capabilities and features
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct ModelCapabilities {
    pub supports_streaming: bool,
    pub supports_function_calling: bool,
    pub supports_vision: bool,
    pub supports_code_execution: bool,
    pub input_modalities: Vec<String>,
    pub output_modalities: Vec<String>,
    pub languages: Vec<String>,
    pub specializations: Vec<String>,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            supports_streaming: true,
            supports_function_calling: false,
            supports_vision: false,
            supports_code_execution: false,
            input_modalities: vec!["text".to_string()],
            output_modalities: vec!["text".to_string()],
            languages: vec!["en".to_string()],
            specializations: vec![],
        }
    }
}

/// Model registry for tracking available models across providers
pub struct ModelRegistry {
    models: Arc<RwLock<HashMap<String, ModelInfo>>>,
    storage: Arc<MemoryStorage>,
}

impl ModelRegistry {
    /// Create a new model registry
    pub async fn new() -> Result<Self> {
        let storage = Arc::new(MemoryStorage::new().await?);
        let registry = Self {
            models: Arc::new(RwLock::new(HashMap::new())),
            storage,
        };

        // Load existing models from storage
        registry.load_from_storage().await?;

        Ok(registry)
    }

    /// Register a new model
    pub async fn register_model(&self, model: ModelInfo) -> Result<()> {
        let model_key = format!("{}:{}", model.provider_name, model.model_id);

        // Update in-memory registry
        {
            let mut models = self.models.write().await;
            models.insert(model_key.clone(), model.clone());
        }

        // Persist to storage
        self.persist_model(&model).await?;

        Ok(())
    }

    /// Get model information
    pub async fn get_model(&self, provider: &str, model_id: &str) -> Option<ModelInfo> {
        let model_key = format!("{provider}:{model_id}");
        let models = self.models.read().await;
        models.get(&model_key).cloned()
    }

    /// List all models for a provider
    pub async fn list_provider_models(&self, provider: &str) -> Vec<ModelInfo> {
        let models = self.models.read().await;
        models
            .values()
            .filter(|m| m.provider_name == provider)
            .cloned()
            .collect()
    }

    /// Search models by capability
    pub async fn search_by_capability(&self, capability: &str) -> Vec<ModelInfo> {
        let models = self.models.read().await;
        models
            .values()
            .filter(|m| match capability {
                "streaming" => m.capabilities.supports_streaming,
                "function_calling" => m.capabilities.supports_function_calling,
                "vision" => m.capabilities.supports_vision,
                "code_execution" => m.capabilities.supports_code_execution,
                spec => m.capabilities.specializations.contains(&spec.to_string()),
            })
            .cloned()
            .collect()
    }

    /// Find models suitable for a task type
    pub async fn find_models_for_task(&self, task_type: &str) -> Vec<ModelInfo> {
        let models = self.models.read().await;

        let suitable_models: Vec<ModelInfo> = models
            .values()
            .filter(|m| {
                match task_type {
                    "code_review" => {
                        m.tags.iter().any(|t| t == "code" || t == "programming")
                            || m.capabilities.specializations.contains(&"code".to_string())
                    }
                    "summarization" => m
                        .tags
                        .iter()
                        .any(|t| t == "summarization" || t == "general"),
                    "translation" => {
                        m.tags
                            .iter()
                            .any(|t| t == "translation" || t == "multilingual")
                            || m.capabilities.languages.len() > 1
                    }
                    "vision" => m.capabilities.supports_vision,
                    _ => true, // Return all models for unknown tasks
                }
            })
            .cloned()
            .collect();

        suitable_models
    }

    /// Update model information
    pub async fn update_model(&self, model: ModelInfo) -> Result<()> {
        let model_key = format!("{}:{}", model.provider_name, model.model_id);

        // Update in-memory registry
        {
            let mut models = self.models.write().await;
            models.insert(model_key, model.clone());
        }

        // Update in storage
        self.persist_model(&model).await?;

        Ok(())
    }

    /// Remove a model from registry
    pub async fn remove_model(&self, provider: &str, model_id: &str) -> Result<()> {
        let model_key = format!("{provider}:{model_id}");

        // Remove from in-memory registry
        {
            let mut models = self.models.write().await;
            models.remove(&model_key);
        }

        // Mark as removed in storage (we don't actually delete, just mark)
        // Create search criteria
        let search_text = format!("{provider} {model_id}");
        let results = self
            .storage
            .search(&search_text, 1, Some("model_info"))
            .await?;

        if let Some(result) = results.first() {
            if let Some(mut metadata) = result.memory.metadata.clone() {
                metadata["removed"] = json!(true);
                metadata["removed_at"] = json!(Utc::now().to_rfc3339());

                // Update the memory with removed flag
                let updated_memory = arkavo_memory::models::Memory {
                    id: result.memory.id,
                    content: result.memory.content.clone(),
                    embedding: result.memory.embedding.clone(),
                    category: result.memory.category.clone(),
                    metadata: Some(metadata),
                    created_at: result.memory.created_at,
                    updated_at: Utc::now(),
                };

                self.storage.store(updated_memory).await?;
            }
        }

        Ok(())
    }

    /// Load models from storage
    async fn load_from_storage(&self) -> Result<()> {
        // Load all models
        let results = self.storage.search("", 100, Some("model_info")).await?;

        {
            let mut models = self.models.write().await;

            for result in results {
                if let Ok(model) = serde_json::from_str::<ModelInfo>(&result.memory.content) {
                    let model_key = format!("{}:{}", model.provider_name, model.model_id);
                    models.insert(model_key, model);
                }
            }
        }

        Ok(())
    }

    /// Persist a model to storage
    async fn persist_model(&self, model: &ModelInfo) -> Result<()> {
        let content = serde_json::to_string_pretty(model)?;

        let memory = arkavo_memory::models::Memory {
            id: uuid::Uuid::new_v4(),
            content,
            embedding: vec![],
            category: Some("model_info".to_string()),
            metadata: Some(json!({
                "type": "model_info",
                "provider": model.provider_name,
                "model_id": model.model_id,
                "display_name": model.display_name,
                "tags": model.tags,
                "version": "1.0"
            })),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        self.storage.store(memory).await?;

        Ok(())
    }

    /// Export registry to JSON
    pub async fn export_to_json(&self) -> Result<String> {
        let export = {
            let models = self.models.read().await;
            json!({
                "version": "1.0",
                "exported_at": Utc::now().to_rfc3339(),
                "model_count": models.len(),
                "models": models.values().collect::<Vec<_>>()
            })
        };

        Ok(serde_json::to_string_pretty(&export)?)
    }

    /// Import models from JSON
    pub async fn import_from_json(&self, json_data: &str) -> Result<u32> {
        let import_data: serde_json::Value = serde_json::from_str(json_data)?;
        let models = import_data["models"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Invalid import format: missing models array"))?;

        let mut imported_count = 0;

        for model_json in models {
            if let Ok(model) = serde_json::from_value::<ModelInfo>(model_json.clone()) {
                self.register_model(model).await?;
                imported_count += 1;
            }
        }

        Ok(imported_count)
    }

    /// Update pricing information for a model
    pub async fn update_model_pricing(
        &self,
        provider: &str,
        model_id: &str,
        pricing: ModelPricing,
    ) -> Result<()> {
        let model_key = format!("{provider}:{model_id}");
        let models = self.models.read().await;

        if let Some(mut model) = models.get(&model_key).cloned() {
            drop(models);
            model.pricing = Some(pricing);
            model.updated_at = Utc::now();
            self.update_model(model).await?;
        }

        Ok(())
    }
}

/// Pre-defined model configurations for common models
pub fn get_default_models() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            model_id: "llama3.2:latest".to_string(),
            provider_name: "ollama".to_string(),
            display_name: "Llama 3.2".to_string(),
            description: Some("Latest Llama 3.2 model with improved performance".to_string()),
            context_window: Some(8192),
            max_output_tokens: Some(4096),
            capabilities: ModelCapabilities {
                supports_streaming: true,
                supports_function_calling: true,
                ..Default::default()
            },
            tags: vec!["general".to_string(), "chat".to_string()],
            version: Some("3.2".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: None,
            pricing: Some(ModelPricing {
                input_cost_per_thousand_cents: 0,
                output_cost_per_thousand_cents: 0,
                effective_date: Utc::now(),
                currency: "USD".to_string(),
            }),
        },
        ModelInfo {
            model_id: "devstral:latest".to_string(),
            provider_name: "ollama".to_string(),
            display_name: "Devstral".to_string(),
            description: Some("Code-focused model optimized for programming tasks".to_string()),
            context_window: Some(32768),
            max_output_tokens: Some(8192),
            capabilities: ModelCapabilities {
                supports_streaming: true,
                specializations: vec!["code".to_string(), "programming".to_string()],
                ..Default::default()
            },
            tags: vec!["code".to_string(), "programming".to_string()],
            version: Some("latest".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: None,
            pricing: Some(ModelPricing {
                input_cost_per_thousand_cents: 0,
                output_cost_per_thousand_cents: 0,
                effective_date: Utc::now(),
                currency: "USD".to_string(),
            }),
        },
        ModelInfo {
            model_id: "qwen3:0.6b".to_string(),
            provider_name: "ollama".to_string(),
            display_name: "Qwen 3 (0.6B)".to_string(),
            description: Some("Small, fast model for classification and simple tasks".to_string()),
            context_window: Some(4096),
            max_output_tokens: Some(2048),
            capabilities: ModelCapabilities {
                supports_streaming: true,
                languages: vec!["en".to_string(), "zh".to_string()],
                ..Default::default()
            },
            tags: vec![
                "small".to_string(),
                "fast".to_string(),
                "classification".to_string(),
            ],
            version: Some("0.6b".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: None,
            pricing: Some(ModelPricing {
                input_cost_per_thousand_cents: 0,
                output_cost_per_thousand_cents: 0,
                effective_date: Utc::now(),
                currency: "USD".to_string(),
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_model_registry() {
        // Note: This test would need a test database setup
        // For now, we'll test the non-storage parts

        let default_models = get_default_models();
        assert!(!default_models.is_empty());

        // Test model capabilities
        let llama = &default_models[0];
        assert!(llama.capabilities.supports_streaming);
        assert_eq!(llama.provider_name, "ollama");

        // Test finding code models
        let code_models: Vec<_> = default_models
            .iter()
            .filter(|m| m.tags.contains(&"code".to_string()))
            .collect();
        assert!(!code_models.is_empty());
    }

    #[test]
    fn test_model_capabilities_default() {
        let caps = ModelCapabilities::default();
        assert!(caps.supports_streaming);
        assert!(!caps.supports_vision);
        assert_eq!(caps.input_modalities, vec!["text"]);
    }
}
