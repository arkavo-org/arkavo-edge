use crate::nodes::llm_config::{
    LlmConfigBuilder, LlmConfiguration, load_llm_config, store_llm_config,
};
use crate::nodes::llm_discovery::{
    discover_ollama_providers, get_llm_capability_info, suggest_llm_node_config,
};
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
        let config_json = store_llm_config(&config)?;

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
        store_llm_config(&config)?;

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

#[cfg(test)]
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
    }
}
