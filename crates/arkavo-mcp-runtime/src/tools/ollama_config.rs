use arkavo_mcp::{Tool, ToolSchema};
use arkavo_memory::models::Memory;
use arkavo_memory::storage::MemoryStorage;
use async_trait::async_trait;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

const OLLAMA_SERVER_CONFIG_TYPE: &str = "arkavo_ollama_server_config";
const CONFIG_CATEGORY: &str = "config";

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

fn normalize_server_url(server_url: &str) -> String {
    let trimmed = server_url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

fn validate_ollama_server_url(server_url: &str) -> Result<String, String> {
    let normalized = normalize_server_url(server_url);
    let parsed = Url::parse(&normalized)
        .map_err(|e| format!("Invalid Ollama server URL '{normalized}': {e}"))?;

    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Ollama server URL must use http:// or https://".to_string());
    }

    match parsed.host_str() {
        Some(host) if arkavo_validation::is_loopback_host(host) => Ok(normalized),
        Some(_) => Err(
            "Remote Ollama server URLs are blocked for security. Use localhost or loopback IP."
                .to_string(),
        ),
        None => Err("Invalid Ollama server URL: missing host".to_string()),
    }
}

fn is_ollama_config(memory: &Memory) -> bool {
    memory
        .metadata
        .as_ref()
        .and_then(|m| m.get("type"))
        .and_then(Value::as_str)
        == Some(OLLAMA_SERVER_CONFIG_TYPE)
}

async fn load_persisted_configs(
    storage: &MemoryStorage,
) -> Result<Vec<Memory>, Box<dyn std::error::Error + Send + Sync>> {
    let configs = storage
        .search(OLLAMA_SERVER_CONFIG_TYPE, 200, Some(CONFIG_CATEGORY))
        .await?;
    Ok(configs
        .into_iter()
        .map(|result| result.memory)
        .filter(is_ollama_config)
        .collect())
}

async fn remove_persisted_server(
    storage: &MemoryStorage,
    target_url: &str,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let normalized_target = normalize_server_url(target_url);
    let configs = load_persisted_configs(storage).await?;
    let mut removed_count = 0usize;

    for memory in configs {
        if normalize_server_url(&memory.content) == normalized_target {
            storage.delete(memory.id).await?;
            removed_count += 1;
        }
    }

    Ok(removed_count)
}

#[async_trait]
impl Tool for OllamaConfigTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let config_params: OllamaConfigParams = serde_json::from_value(params)?;

        match config_params.action.as_str() {
            "add" => {
                if let Some(server_url) = config_params.server_url {
                    // Validate and normalize URL
                    let url = match validate_ollama_server_url(&server_url) {
                        Ok(url) => url,
                        Err(error) => {
                            return Ok(json!({
                                "success": false,
                                "error": error,
                            }));
                        }
                    };

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
                                    "type": OLLAMA_SERVER_CONFIG_TYPE,
                                    "timestamp": chrono::Utc::now().to_rfc3339(),
                                    "model_count": models.len(),
                                })),
                                category: Some(CONFIG_CATEGORY.to_string()),
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
                let configs = load_persisted_configs(&storage).await?;

                let servers: Vec<_> = configs
                    .into_iter()
                    .map(|memory| {
                        json!({
                            "url": memory.content,
                            "added_at": memory.created_at.to_rfc3339(),
                        })
                    })
                    .collect();

                Ok(json!({
                    "success": true,
                    "servers": servers,
                }))
            }
            "remove" => {
                let Some(server_url) = config_params.server_url.as_deref() else {
                    return Ok(json!({
                        "success": false,
                        "error": "Server URL is required for 'remove' action",
                    }));
                };

                let storage = Arc::new(MemoryStorage::new().await?);
                let removed_count = remove_persisted_server(&storage, server_url).await?;

                // Extract server name from URL or use default
                let server_name = if server_url.contains("://") {
                    // Parse URL to get host as name
                    server_url
                        .split("://")
                        .nth(1)
                        .and_then(|s| s.split(':').next())
                        .unwrap_or("default")
                } else {
                    "default"
                };

                Ok(json!({
                    "success": true,
                    "message": format!("Removed {} persisted config(s) for server '{}'", removed_count, server_name),
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
                aliases: None,
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
                            "description": "The Ollama server URL. For security, only localhost/loopback URLs are allowed (e.g., 'localhost:11434' or 'http://127.0.0.1:11434'). Required for 'add' and 'remove' actions."
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

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    #[tokio::test]
    async fn remove_deletes_matching_persisted_ollama_configs() {
        let storage = MemoryStorage::new_test()
            .await
            .expect("storage should initialize");
        let now = chrono::Utc::now();

        let matching = Memory {
            id: Uuid::new_v4(),
            content: "http://127.0.0.1:11434".to_string(),
            metadata: Some(json!({ "type": OLLAMA_SERVER_CONFIG_TYPE })),
            category: Some(CONFIG_CATEGORY.to_string()),
            embedding: Vec::new(),
            created_at: now,
            updated_at: now,
        };

        let non_matching = Memory {
            id: Uuid::new_v4(),
            content: "http://127.0.0.1:11435".to_string(),
            metadata: Some(json!({ "type": OLLAMA_SERVER_CONFIG_TYPE })),
            category: Some(CONFIG_CATEGORY.to_string()),
            embedding: Vec::new(),
            created_at: now,
            updated_at: now,
        };

        storage
            .store(matching.clone())
            .await
            .expect("matching config should persist");
        storage
            .store(non_matching.clone())
            .await
            .expect("non-matching config should persist");

        let removed = remove_persisted_server(&storage, "127.0.0.1:11434")
            .await
            .expect("remove should succeed");
        assert_eq!(removed, 1);

        let remaining = load_persisted_configs(&storage)
            .await
            .expect("list should succeed");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].content, non_matching.content);
    }

    #[test]
    fn validate_ollama_url_allows_loopback_hosts() {
        assert!(validate_ollama_server_url("localhost:11434").is_ok());
        assert!(validate_ollama_server_url("http://127.0.0.1:11434").is_ok());
        assert!(validate_ollama_server_url("http://[::1]:11434").is_ok());
    }

    #[test]
    fn validate_ollama_url_blocks_non_loopback_hosts() {
        let private_ip = validate_ollama_server_url("http://192.168.1.100:11434");
        assert!(private_ip.is_err());

        let public_domain = validate_ollama_server_url("https://example.com:11434");
        assert!(public_domain.is_err());
    }
}
