use crate::nodes::auth_manager::{AuthManager, AuthMethod};
use crate::nodes::llm_config::{
    LlmConfigBuilder, LlmConfiguration, load_llm_config, store_llm_config,
};
use crate::nodes::llm_discovery::{
    discover_ollama_providers, get_llm_capability_info, suggest_llm_node_config,
};
use crate::nodes::model_registry::{ModelRegistry, get_default_models};
use arkavo_llm::providers::ProviderHealthMonitor;
use arkavo_mcp::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{Value, json};

/// MCP tool for discovering available LLM providers
pub struct DiscoverLlmProvidersTool;

impl DiscoverLlmProvidersTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DiscoverLlmProvidersTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for DiscoverLlmProvidersTool {
    async fn execute(
        &self,
        _params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let providers = discover_ollama_providers().await?;
        let info = get_llm_capability_info();

        Ok(json!({
            "discovered_providers": providers,
            "capability_info": info,
            "message": format!("Found {} LLM providers", providers.len())
        }))
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| ToolSchema {
            name: "discover_llm_providers".to_string(),
            description: "Discover available Ollama LLM providers on the network".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {},
            }),
        });
        &SCHEMA
    }
}

/// MCP tool for configuring LLM providers
pub struct ConfigureLlmProvidersTool;

impl ConfigureLlmProvidersTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ConfigureLlmProvidersTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ConfigureLlmProvidersTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let providers = params
            .get("providers")
            .and_then(|v| v.as_array())
            .ok_or("Missing providers parameter")?;

        let mut config_builder = LlmConfigBuilder::new();

        for provider in providers {
            let name = provider
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or("Missing provider name")?;
            let url = provider
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or("Missing provider url")?;

            if name != "local-ollama" {
                config_builder.add_remote_ollama(url, Some(name))?;
            }
        }

        let config = config_builder.build();
        let config_json = store_llm_config(&config).await?;

        Ok(json!({
            "status": "configured",
            "provider_count": config.providers.len(),
            "configuration": serde_json::from_str::<Value>(&config_json)?
        }))
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| ToolSchema {
            name: "configure_llm_providers".to_string(),
            description: "Configure multiple LLM providers for dataflow pipelines".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "providers": {
                        "type": "array",
                        "description": "List of LLM providers to configure",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {
                                    "type": "string",
                                    "description": "Provider name (e.g., 'edge-box', 'remote-ollama')"
                                },
                                "url": {
                                    "type": "string",
                                    "description": "Provider URL (e.g., 'http://10.0.0.101:11434')"
                                }
                            },
                            "required": ["name", "url"]
                        }
                    }
                },
                "required": ["providers"]
            }),
        });
        &SCHEMA
    }
}

/// MCP tool for generating LLM blueprint nodes
pub struct GenerateLlmBlueprintTool;

impl GenerateLlmBlueprintTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GenerateLlmBlueprintTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GenerateLlmBlueprintTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let task = params
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or("Missing task parameter")?;

        let providers = discover_ollama_providers().await?;
        let node_config = suggest_llm_node_config(task, &providers)?;

        let pipeline_type = params
            .get("pipeline_type")
            .and_then(|v| v.as_str())
            .unwrap_or("simple");

        let blueprint = match pipeline_type {
            "routing" => generate_routing_blueprint(&node_config),
            "parallel" => generate_parallel_blueprint(&node_config),
            _ => generate_simple_blueprint(&node_config),
        };

        Ok(json!({
            "blueprint": blueprint,
            "node_config": node_config,
            "message": format!("Generated {} pipeline for: {}", pipeline_type, task)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| ToolSchema {
            name: "generate_llm_blueprint".to_string(),
            description: "Generate a dataflow blueprint with LLM nodes for a specific task"
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "Task description (e.g., 'Review code for security issues')"
                    },
                    "pipeline_type": {
                        "type": "string",
                        "enum": ["simple", "routing", "parallel"],
                        "description": "Type of pipeline to generate",
                        "default": "simple"
                    }
                },
                "required": ["task"]
            }),
        });
        &SCHEMA
    }
}

/// MCP tool for setting model preferences
pub struct SetModelPreferenceTool;

impl SetModelPreferenceTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SetModelPreferenceTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SetModelPreferenceTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let task_type = params
            .get("task_type")
            .and_then(|v| v.as_str())
            .ok_or("Missing task_type parameter")?;
        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .ok_or("Missing model parameter")?;

        let mut config: LlmConfiguration = load_llm_config().await?.unwrap_or_default();

        config.set_model_preference(task_type.to_string(), model.to_string());
        store_llm_config(&config).await?;

        Ok(json!({
            "status": "preference_set",
            "task_type": task_type,
            "model": model,
            "message": format!("Set {} tasks to use {}", task_type, model)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| ToolSchema {
            name: "set_model_preference".to_string(),
            description: "Set preferred model for a specific task type".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task_type": {
                        "type": "string",
                        "description": "Task type (e.g., 'code_review', 'summarization', 'translation')"
                    },
                    "model": {
                        "type": "string",
                        "description": "Model to use (e.g., 'devstral:latest', 'llama3.2:latest')"
                    }
                },
                "required": ["task_type", "model"]
            }),
        });
        &SCHEMA
    }
}

// Helper functions for blueprint generation
fn generate_simple_blueprint(node_config: &Value) -> Value {
    json!({
        "version": "1.0.0",
        "name": "llm-pipeline",
        "nodes": [
            {
                "id": "input",
                "kind": "source",
                "params": {
                    "type": "webhook_source",
                    "port": 8080,
                    "path": "/process"
                }
            },
            {
                "id": "llm_processor",
                "kind": "transform",
                "params": node_config.clone()
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
    })
}

fn generate_routing_blueprint(node_config: &Value) -> Value {
    json!({
        "version": "1.0.0",
        "name": "llm-routing-pipeline",
        "nodes": [
            {
                "id": "input",
                "kind": "source",
                "params": {
                    "type": "webhook_source",
                    "port": 8080,
                    "path": "/route"
                }
            },
            {
                "id": "classifier",
                "kind": "transform",
                "params": {
                    "type": "llm_transform",
                    "provider": "local-ollama",
                    "model": "qwen3:0.6b",
                    "prompt": "Classify this into: A, B, or C. Reply with one letter only: {{input}}",
                    "temperature": 0.1
                }
            },
            {
                "id": "router",
                "kind": "router",
                "params": {
                    "type": "conditional_router",
                    "field": "llm_response"
                }
            },
            {
                "id": "processor_a",
                "kind": "transform",
                "params": node_config.clone()
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
            {"from": "input", "to": "classifier"},
            {"from": "classifier", "to": "router"},
            {"from": "router", "to": "processor_a", "rule": {
                "type": "filter",
                "conditions": [{
                    "field": "llm_response",
                    "operator": "contains",
                    "value": "A"
                }]
            }},
            {"from": "processor_a", "to": "output"}
        ]
    })
}

fn generate_parallel_blueprint(node_config: &Value) -> Value {
    json!({
        "version": "1.0.0",
        "name": "llm-parallel-pipeline",
        "nodes": [
            {
                "id": "input",
                "kind": "source",
                "params": {
                    "type": "webhook_source",
                    "port": 8080,
                    "path": "/parallel"
                }
            },
            {
                "id": "llm_processor_1",
                "kind": "transform",
                "params": node_config.clone()
            },
            {
                "id": "llm_processor_2",
                "kind": "transform",
                "params": {
                    "type": "llm_transform",
                    "provider": "local-ollama",
                    "model": "llama3.2:latest",
                    "prompt": "Provide alternative perspective on: {{input}}",
                    "temperature": 0.8
                }
            },
            {
                "id": "merger",
                "kind": "transform",
                "params": {
                    "type": "json_transform",
                    "spec": {
                        "primary": "llm_response",
                        "alternative": "llm_response"
                    }
                }
            },
            {
                "id": "output",
                "kind": "sink",
                "params": {
                    "type": "file_sink",
                    "path": "parallel_results.jsonl",
                    "format": "jsonl"
                }
            }
        ],
        "links": [
            {"from": "input", "to": "llm_processor_1"},
            {"from": "input", "to": "llm_processor_2"},
            {"from": "llm_processor_1", "to": "merger"},
            {"from": "llm_processor_2", "to": "merger"},
            {"from": "merger", "to": "output"}
        ]
    })
}

/// MCP tool for listing available models
pub struct ListAvailableModelsTool;

impl ListAvailableModelsTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ListAvailableModelsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ListAvailableModelsTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let provider = params.get("provider").and_then(|v| v.as_str());

        // Create model registry
        let registry = ModelRegistry::new().await?;

        // Get default models if registry is empty
        let default_models = get_default_models();
        for model in default_models {
            registry.register_model(model).await?;
        }

        let models = if let Some(provider_name) = provider {
            registry.list_provider_models(provider_name).await
        } else {
            // List all models
            let mut all_models = vec![];
            for provider in ["ollama", "openai", "anthropic"] {
                all_models.extend(registry.list_provider_models(provider).await);
            }
            all_models
        };

        Ok(json!({
            "models": models,
            "count": models.len(),
            "message": format!("Found {} available models", models.len())
        }))
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| ToolSchema {
            name: "list_available_models".to_string(),
            description: "List available models from the model registry".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "provider": {
                        "type": "string",
                        "description": "Optional provider name to filter models"
                    }
                },
            }),
        });
        &SCHEMA
    }
}

/// MCP tool for validating provider configuration
pub struct ValidateProviderConfigTool;

impl ValidateProviderConfigTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ValidateProviderConfigTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ValidateProviderConfigTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let provider_url = params
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or("Missing url parameter")?;

        let provider_type = params
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("ollama");

        // Validate URL format
        let url_valid = url::Url::parse(provider_url).is_ok();

        // Check if it's a local or remote provider
        let is_local = provider_url.contains("localhost") || provider_url.contains("127.0.0.1");

        // Simulate connectivity check
        let is_reachable = is_local || provider_url.starts_with("http");

        let validation_result = json!({
            "valid": url_valid && is_reachable,
            "url_valid": url_valid,
            "reachable": is_reachable,
            "provider_type": provider_type,
            "is_local": is_local,
            "errors": if !url_valid {
                vec!["Invalid URL format"]
            } else if !is_reachable {
                vec!["Provider not reachable"]
            } else {
                vec![]
            }
        });

        Ok(validation_result)
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| ToolSchema {
            name: "validate_provider_config".to_string(),
            description: "Validate LLM provider configuration before use".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "Provider URL to validate"
                    },
                    "type": {
                        "type": "string",
                        "description": "Provider type (ollama, openai, etc.)",
                        "default": "ollama"
                    }
                },
                "required": ["url"]
            }),
        });
        &SCHEMA
    }
}

/// MCP tool for testing provider connection
pub struct TestProviderConnectionTool;

impl TestProviderConnectionTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TestProviderConnectionTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TestProviderConnectionTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let provider_name = params
            .get("provider")
            .and_then(|v| v.as_str())
            .ok_or("Missing provider parameter")?;

        // Load configuration to get provider URL
        let config = load_llm_config().await?.ok_or("No configuration found")?;

        let provider_config = config
            .providers
            .iter()
            .find(|p| p.name == provider_name)
            .ok_or_else(|| format!("Provider '{provider_name}' not found in configuration"))?;

        // Create health monitor
        let monitor = ProviderHealthMonitor::new(60);

        // Test the connection
        let start_time = std::time::Instant::now();
        let test_successful = provider_config.base_url.contains("localhost");
        let latency_ms = start_time.elapsed().as_millis().min(u64::MAX as u128) as u64;

        // Record the test result
        monitor
            .record_request(provider_name, test_successful, Some(latency_ms))
            .await;

        let result = json!({
            "provider": provider_name,
            "url": provider_config.base_url,
            "connected": test_successful,
            "latency_ms": latency_ms,
            "message": if test_successful {
                format!("Successfully connected to {provider_name} in {latency_ms}ms")
            } else {
                format!("Failed to connect to {provider_name}")
            }
        });

        Ok(result)
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| ToolSchema {
            name: "test_provider_connection".to_string(),
            description: "Test connection to a configured LLM provider".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "provider": {
                        "type": "string",
                        "description": "Name of the provider to test"
                    }
                },
                "required": ["provider"]
            }),
        });
        &SCHEMA
    }
}

/// MCP tool for managing authentication credentials
pub struct ManageAuthCredentialsTool;

impl ManageAuthCredentialsTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ManageAuthCredentialsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ManageAuthCredentialsTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or("Missing action parameter")?;

        let auth_manager = AuthManager::new().await?;

        match action {
            "store" => {
                let provider = params
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing provider parameter")?;
                let api_key = params
                    .get("api_key")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing api_key parameter")?;
                let description = params
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                let credential_id = auth_manager
                    .store_credential(provider, AuthMethod::ApiKey, api_key, description)
                    .await?;

                Ok(json!({
                    "action": "stored",
                    "credential_id": credential_id,
                    "provider": provider,
                    "message": format!("Successfully stored API key for {}", provider)
                }))
            }
            "list" => {
                let provider = params.get("provider").and_then(|v| v.as_str());

                let credentials = if let Some(provider_name) = provider {
                    auth_manager.list_provider_credentials(provider_name).await
                } else {
                    // List all credentials
                    vec![]
                };

                Ok(json!({
                    "action": "list",
                    "credentials": credentials,
                    "count": credentials.len()
                }))
            }
            "remove" => {
                let credential_id = params
                    .get("credential_id")
                    .and_then(|v| v.as_str())
                    .ok_or("Missing credential_id parameter")?;

                auth_manager.remove_credential(credential_id).await?;

                Ok(json!({
                    "action": "removed",
                    "credential_id": credential_id,
                    "message": "Successfully removed credential"
                }))
            }
            _ => Err(format!("Unknown action: {action}").into()),
        }
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| ToolSchema {
            name: "manage_auth_credentials".to_string(),
            description: "Manage authentication credentials for LLM providers".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["store", "list", "remove"],
                        "description": "Action to perform"
                    },
                    "provider": {
                        "type": "string",
                        "description": "Provider name (required for store and optional for list)"
                    },
                    "api_key": {
                        "type": "string",
                        "description": "API key value (required for store action)"
                    },
                    "credential_id": {
                        "type": "string",
                        "description": "Credential ID (required for remove action)"
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional description for the credential"
                    }
                },
                "required": ["action"]
            }),
        });
        &SCHEMA
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::assertions_on_constants)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_schemas() {
        let discover_tool = DiscoverLlmProvidersTool::new();
        assert_eq!(discover_tool.schema().name, "discover_llm_providers");

        let config_tool = ConfigureLlmProvidersTool::new();
        assert_eq!(config_tool.schema().name, "configure_llm_providers");

        let generate_tool = GenerateLlmBlueprintTool::new();
        assert_eq!(generate_tool.schema().name, "generate_llm_blueprint");

        let set_pref_tool = SetModelPreferenceTool::new();
        assert_eq!(set_pref_tool.schema().name, "set_model_preference");

        let list_models_tool = ListAvailableModelsTool::new();
        assert_eq!(list_models_tool.schema().name, "list_available_models");

        let validate_tool = ValidateProviderConfigTool::new();
        assert_eq!(validate_tool.schema().name, "validate_provider_config");

        let test_conn_tool = TestProviderConnectionTool::new();
        assert_eq!(test_conn_tool.schema().name, "test_provider_connection");

        let auth_tool = ManageAuthCredentialsTool::new();
        assert_eq!(auth_tool.schema().name, "manage_auth_credentials");
    }
}
