use crate::nodes::llm_discovery::{discover_ollama_providers, get_llm_capability_info, suggest_llm_node_config};
use crate::nodes::llm_config::{LlmConfiguration, LlmConfigBuilder, store_llm_config, load_llm_config};
use anyhow::Result;
use serde_json::json;

/// Agent interface for LLM dataflow configuration
/// This module provides methods for AI agents to discover, configure, and use LLM capabilities
pub struct LlmDataflowAgent;

impl LlmDataflowAgent {
    /// Discover what LLM capabilities are available
    pub async fn discover_capabilities(&self) -> Result<serde_json::Value> {
        let info = get_llm_capability_info();
        let discovered_providers = discover_ollama_providers().await?;
        
        Ok(json!({
            "available_providers": discovered_providers,
            "capability_info": info,
            "instructions": {
                "discovery": "Use discover_ollama_providers() to find available Ollama instances",
                "configuration": "Use configure_providers() to set up multiple Ollama endpoints",
                "usage": "Create llm_transform nodes in blueprints with appropriate provider and model",
                "storage": "Configuration is automatically stored in memory for persistence"
            }
        }))
    }
    
    /// Configure LLM providers based on discovered or known endpoints
    pub async fn configure_providers(&self, providers: Vec<(String, String)>) -> Result<String> {
        let mut config_builder = LlmConfigBuilder::new();
        
        for (name, url) in providers {
            if name != "local-ollama" { // local-ollama is added by default
                config_builder.add_remote_ollama(&url, Some(&name))?;
            }
        }
        
        let config = config_builder.build();
        
        // Store configuration
        let _stored = store_llm_config(&config)?;
        
        Ok(format!(
            "Configured {} providers. Configuration stored in memory.",
            config.providers.len()
        ))
    }
    
    /// Get current LLM configuration
    pub async fn get_current_config(&self) -> Result<serde_json::Value> {
        if let Some(config) = load_llm_config().await? {
            Ok(serde_json::to_value(config)?)
        } else {
            Ok(json!({
                "status": "no_stored_config",
                "message": "No configuration found. Using defaults.",
                "default": {
                    "providers": [{
                        "name": "local-ollama",
                        "base_url": "http://localhost:11434"
                    }]
                }
            }))
        }
    }
    
    /// Suggest a blueprint configuration for a given task
    pub async fn suggest_blueprint_for_task(&self, task_description: &str) -> Result<serde_json::Value> {
        let providers = discover_ollama_providers().await?;
        let node_config = suggest_llm_node_config(task_description, &providers)?;
        
        Ok(json!({
            "suggested_node": node_config,
            "complete_blueprint": {
                "version": "1.0.0",
                "name": "ai-generated-pipeline",
                "nodes": [
                    {
                        "id": "input",
                        "kind": "source",
                        "params": {
                            "type": "webhook_source",
                            "port": 8080,
                            "path": "/data"
                        }
                    },
                    {
                        "id": "llm_processor",
                        "kind": "transform",
                        "params": node_config
                    },
                    {
                        "id": "output",
                        "kind": "sink",
                        "params": {
                            "type": "console_sink",
                            "format": "json"
                        }
                    }
                ],
                "links": [
                    {"from": "input", "to": "llm_processor"},
                    {"from": "llm_processor", "to": "output"}
                ]
            }
        }))
    }
    
    /// Update model preferences for specific task types
    pub async fn set_task_model_preference(&self, task_type: &str, model: &str) -> Result<String> {
        let mut config = if let Some(existing) = load_llm_config().await? {
            existing
        } else {
            LlmConfiguration::new()
        };
        
        config.set_model_preference(task_type.to_string(), model.to_string());
        store_llm_config(&config)?;
        
        Ok(format!("Set model preference for task '{}' to '{}'", task_type, model))
    }
    
    /// Get example usage patterns
    pub fn get_usage_examples(&self) -> serde_json::Value {
        json!({
            "discovery_example": {
                "description": "Find available Ollama instances",
                "code": "let providers = discover_ollama_providers().await?;"
            },
            "configuration_example": {
                "description": "Configure multiple providers",
                "code": r#"configure_providers(vec![
                    ("local-ollama", "http://localhost:11434"),
                    ("edge-box", "http://10.0.0.101:11434")
                ]).await?"#
            },
            "blueprint_example": {
                "description": "Generate blueprint for code review",
                "code": r#"suggest_blueprint_for_task("Review Python code for security issues").await?"#
            },
            "preference_example": {
                "description": "Set model preference for code review",
                "code": r#"set_task_model_preference("code_review", "devstral:latest").await?"#
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discover_capabilities() {
        let agent = LlmDataflowAgent;
        let result = agent.discover_capabilities().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_get_usage_examples() {
        let agent = LlmDataflowAgent;
        let examples = agent.get_usage_examples();
        assert!(examples.get("discovery_example").is_some());
    }
}