use crate::error::MemoryError;
use crate::models::{CreateMemoryRequest, Memory};
use crate::storage::MemoryStorage;
use arkavo_mcp::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

pub struct StoreMemoryTool {
    schema: ToolSchema,
    storage: Arc<MemoryStorage>,
}

impl StoreMemoryTool {
    pub fn new(storage: Arc<MemoryStorage>) -> Self {
        Self {
            schema: ToolSchema {
                name: "store_memory".to_string(),
                description: "Store a memory with automatic embedding generation".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The memory content to store"
                        },
                        "metadata": {
                            "type": "object",
                            "description": "Optional metadata as JSON object"
                        },
                        "category": {
                            "type": "string",
                            "description": "Optional category for the memory"
                        }
                    },
                    "required": ["content"]
                }),
            },
            storage,
        }
    }
}

#[async_trait]
impl Tool for StoreMemoryTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Box::new(MemoryError::BadRequest(
                    "Missing content parameter".to_string(),
                )) as Box<dyn std::error::Error + Send + Sync>
            })?
            .to_string();

        let metadata = params.get("metadata").cloned();
        let category = params
            .get("category")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);

        let request = CreateMemoryRequest {
            content,
            metadata,
            category,
        };

        // Generate embedding
        let embedding_service = crate::embeddings::EmbeddingService::new();
        let embedding = embedding_service
            .generate_embedding(&request.content)
            .await
            .map_err(|e| {
                Box::new(MemoryError::Embedding(format!(
                    "Failed to generate embedding: {e}"
                ))) as Box<dyn std::error::Error + Send + Sync>
            })?;

        let memory = Memory {
            id: Uuid::new_v4(),
            content: request.content,
            metadata: request.metadata,
            category: request.category,
            embedding,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let id = memory.id;
        let created_at = memory.created_at;

        self.storage.store(memory).await.map_err(|e| {
            Box::new(MemoryError::Storage(format!("Failed to store memory: {e}")))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

        Ok(serde_json::json!({
            "id": id,
            "created_at": created_at,
            "message": "Memory stored successfully"
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct SearchMemoryTool {
    schema: ToolSchema,
    storage: Arc<MemoryStorage>,
}

impl SearchMemoryTool {
    pub fn new(storage: Arc<MemoryStorage>) -> Self {
        Self {
            schema: ToolSchema {
                name: "search_memory".to_string(),
                description: "Search memories using semantic similarity".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The search query"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of results (default: 10, max: 100)",
                            "minimum": 1,
                            "maximum": 100
                        },
                        "category": {
                            "type": "string",
                            "description": "Optional category filter"
                        }
                    },
                    "required": ["query"]
                }),
            },
            storage,
        }
    }
}

#[async_trait]
impl Tool for SearchMemoryTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Box::new(MemoryError::BadRequest(
                    "Missing query parameter".to_string(),
                )) as Box<dyn std::error::Error + Send + Sync>
            })?;

        let limit = params
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map_or(10, |v| v.try_into().unwrap_or(10))
            .min(100);

        let category = params.get("category").and_then(|v| v.as_str());

        let results = self
            .storage
            .search(query, limit, category)
            .await
            .map_err(|e| {
                Box::new(MemoryError::Storage(format!("Search failed: {e}")))
                    as Box<dyn std::error::Error + Send + Sync>
            })?;

        let total = results.len();
        let results_json: Vec<Value> = results
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "memory": {
                        "id": r.memory.id,
                        "content": r.memory.content,
                        "metadata": r.memory.metadata,
                        "category": r.memory.category,
                        "created_at": r.memory.created_at,
                        "updated_at": r.memory.updated_at,
                    },
                    "score": r.score
                })
            })
            .collect();

        Ok(serde_json::json!({
            "results": results_json,
            "total": total,
            "query": query
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GetMemoryTool {
    schema: ToolSchema,
    storage: Arc<MemoryStorage>,
}

impl GetMemoryTool {
    pub fn new(storage: Arc<MemoryStorage>) -> Self {
        Self {
            schema: ToolSchema {
                name: "get_memory".to_string(),
                description: "Retrieve a specific memory by ID".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The UUID of the memory to retrieve"
                        }
                    },
                    "required": ["id"]
                }),
            },
            storage,
        }
    }
}

#[async_trait]
impl Tool for GetMemoryTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let id_str = params.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
            Box::new(MemoryError::BadRequest("Missing id parameter".to_string()))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

        let id = Uuid::parse_str(id_str).map_err(|e| {
            Box::new(MemoryError::BadRequest(format!("Invalid UUID: {e}")))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

        let memory = self.storage.get(id).await.map_err(|e| {
            Box::new(MemoryError::Storage(format!("Failed to get memory: {e}")))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

        Ok(serde_json::json!({
            "id": memory.id,
            "content": memory.content,
            "metadata": memory.metadata,
            "category": memory.category,
            "created_at": memory.created_at,
            "updated_at": memory.updated_at,
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct CategorizeMemoryTool {
    schema: ToolSchema,
    storage: Arc<MemoryStorage>,
}

impl CategorizeMemoryTool {
    pub fn new(storage: Arc<MemoryStorage>) -> Self {
        Self {
            schema: ToolSchema {
                name: "categorize_memory".to_string(),
                description: "Categorize content based on existing memories".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The content to categorize"
                        }
                    },
                    "required": ["content"]
                }),
            },
            storage,
        }
    }
}

#[async_trait]
impl Tool for CategorizeMemoryTool {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                Box::new(MemoryError::BadRequest(
                    "Missing content parameter".to_string(),
                )) as Box<dyn std::error::Error + Send + Sync>
            })?;

        let (category, confidence) = self.storage.categorize(content).await.map_err(|e| {
            Box::new(MemoryError::Storage(format!("Categorization failed: {e}")))
                as Box<dyn std::error::Error + Send + Sync>
        })?;

        Ok(serde_json::json!({
            "category": category,
            "confidence": confidence,
            "content": content
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
