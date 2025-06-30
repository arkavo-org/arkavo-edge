use anyhow::Result;
use arkavo_mcp_core::{Tool, ToolSchema};
use arkavo_memory::storage::MemoryStorage;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Debug, Deserialize, Serialize)]
pub struct OllamaConfigParams {
    pub action: String, // "add", "remove", "list"
    pub server_url: Option<String>,
}

/// Tool for managing Ollama server configurations
#[derive(Debug)]
pub struct OllamaConfigTool;

impl OllamaConfigTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for OllamaConfigTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let config_params: OllamaConfigParams = serde_json::from_value(params)?;

        match config_params.action.as_str() {
            "add" => {
                if let Some(server_url) = config_params.server_url {
                    // Validate and normalize URL
                    let mut url = server_url.trim().to_string();
                    if !url.starts_with("http://") && !url.starts_with("https://") {
                        url = format!("http://{url}");
                    }

                    // Test connection
                    let client = arkavo_llm::ollama::OllamaClient::new(Some(url.clone()), None);
                    match client.list_models().await {
                        Ok(models) => {
                            // Save configuration
                            let storage = Arc::new(MemoryStorage::new().await?);
                            let embedding = vec![0.0; 384]; // Placeholder embedding

                            let memory = arkavo_memory::models::Memory {
                                id: uuid::Uuid::new_v4(),
                                content: url.clone(),
                                metadata: Some(json!({
                                    "type": "arkavo_ollama_server_config",
                                    "timestamp": chrono::Utc::now().to_rfc3339(),
                                    "model_count": models.len(),
                                })),
                                category: Some("config".to_string()),
                                embedding,
                                created_at: chrono::Utc::now(),
                                updated_at: chrono::Utc::now(),
                            };

                            storage.store(memory).await?;

                            Ok(json!({
                                "success": true,
                                "message": format!("Added Ollama server at {} with {} models", url, models.len()),
                                "models": models,
                            }))
                        }
                        Err(e) => Ok(json!({
                            "success": false,
                            "error": format!("Failed to connect to {}: {}", url, e),
                        })),
                    }
                } else {
                    Ok(json!({
                        "success": false,
                        "error": "Server URL is required for 'add' action",
                    }))
                }
            }
            "list" => {
                let storage = Arc::new(MemoryStorage::new().await?);
                let configs = storage
                    .search("arkavo_ollama_server_config", 20, Some("config"))
                    .await?;

                let servers: Vec<_> = configs
                    .into_iter()
                    .filter(|c| c.memory.content != "CLEARED")
                    .map(|c| {
                        json!({
                            "url": c.memory.content,
                            "added_at": c.memory.created_at.to_rfc3339(),
                        })
                    })
                    .collect();

                Ok(json!({
                    "success": true,
                    "servers": servers,
                }))
            }
            "remove" => {
                // TODO: Implement removal by marking as CLEARED
                Ok(json!({
                    "success": false,
                    "error": "Remove action not yet implemented",
                }))
            }
            _ => Ok(json!({
                "success": false,
                "error": format!("Unknown action: {}", config_params.action),
            })),
        }
    }

    fn schema(&self) -> &ToolSchema {
        static SCHEMA: std::sync::LazyLock<ToolSchema> = std::sync::LazyLock::new(|| {
            ToolSchema {
                name: "ollama_config".to_string(),
                description: "Manage Ollama server configurations. Add new servers, list existing ones, or remove them.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["add", "remove", "list"],
                            "description": "The action to perform"
                        },
                        "server_url": {
                            "type": "string",
                            "description": "The server URL (e.g., '192.168.1.100:11434' or 'http://server.local:11434'). Required for 'add' action."
                        }
                    },
                    "required": ["action"]
                }),
            }
        });
        &SCHEMA
    }
}

impl Default for OllamaConfigTool {
    fn default() -> Self {
        Self::new()
    }
}
