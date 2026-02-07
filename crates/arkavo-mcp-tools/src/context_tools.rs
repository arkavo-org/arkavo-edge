//! RLM Context Tools for MCP integration.
//!
//! Provides context_probe and context_search tools that allow LLMs to
//! interact with decomposed context manifests in RLM mode.
//!
//! These tools use trait objects so the actual RLM implementation
//! can be injected without creating cyclic dependencies.

use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tracing::{debug, info, warn};

use arkavo_mcp::ToolSchema;

use crate::ToolError;
use crate::server::{Tool, error_response, success_response};

/// Trait for RLM context operations.
/// Implemented by arkavo-router::rlm::RlmContextManager.
#[async_trait]
pub trait RlmOperations: Send + Sync {
    /// Decompose context into chunks, returns (manifest_id, chunk_count, total_tokens, previews).
    async fn decompose(&self, content: &str) -> Result<DecomposeResult, String>;

    /// Probe chunks by indices, returns vec of (index, content).
    async fn probe(
        &self,
        manifest_id: &str,
        indices: &[usize],
    ) -> Result<Vec<(usize, String)>, String>;

    /// Search chunks by keywords, returns vec of (index, content).
    async fn search(
        &self,
        manifest_id: &str,
        keywords: &[&str],
    ) -> Result<Vec<(usize, String)>, String>;
}

/// Result from context decomposition.
#[derive(Debug, Clone)]
pub struct DecomposeResult {
    pub manifest_id: String,
    pub chunk_count: usize,
    pub total_tokens: usize,
    pub previews: Vec<ChunkPreview>,
}

/// Preview of a decomposed chunk.
#[derive(Debug, Clone)]
pub struct ChunkPreview {
    pub index: usize,
    pub tokens: usize,
    pub preview: String,
    pub hints: Vec<String>,
}

/// Shared RLM operations reference.
pub type SharedRlmOps = Arc<dyn RlmOperations>;

/// Context probe tool - fetch specific chunks by index.
pub struct ContextProbeTool {
    ops: SharedRlmOps,
    schema: ToolSchema,
}

impl ContextProbeTool {
    /// Create a new context probe tool.
    pub fn new(ops: SharedRlmOps) -> Self {
        Self {
            ops,
            schema: ToolSchema {
                name: "context_probe".to_string(),
                aliases: None,
                description: "Fetch specific chunks from a decomposed context manifest by their indices. Use this to retrieve full content of chunks identified by context_search or from manifest previews.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "manifest_id": {
                            "type": "string",
                            "description": "The manifest ID from the context decomposition"
                        },
                        "indices": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "Array of chunk indices to fetch (0-based)"
                        }
                    },
                    "required": ["manifest_id", "indices"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for ContextProbeTool {
    async fn execute(&self, params: Value) -> crate::Result<Value> {
        let manifest_id = params
            .get("manifest_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("manifest_id is required".to_string()))?;

        let indices: Vec<usize> = params
            .get("indices")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError::InvalidParams("indices array is required".to_string()))?
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as usize))
            .collect();

        if indices.is_empty() {
            return Ok(error_response("No valid indices provided"));
        }

        debug!(manifest_id, indices = ?indices, "Probing chunks");

        match self.ops.probe(manifest_id, &indices).await {
            Ok(chunks) => {
                let results: Vec<Value> = chunks
                    .iter()
                    .map(|(idx, content)| {
                        json!({
                            "index": idx,
                            "content": content
                        })
                    })
                    .collect();

                info!(
                    manifest_id,
                    chunk_count = results.len(),
                    "Chunks probed successfully"
                );

                Ok(success_response(json!({
                    "manifest_id": manifest_id,
                    "chunks": results
                })))
            }
            Err(e) => {
                warn!(manifest_id, error = %e, "Failed to probe chunks");
                Ok(error_response(&e))
            }
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

/// Context search tool - find chunks matching keywords.
pub struct ContextSearchTool {
    ops: SharedRlmOps,
    schema: ToolSchema,
}

impl ContextSearchTool {
    /// Create a new context search tool.
    pub fn new(ops: SharedRlmOps) -> Self {
        Self {
            ops,
            schema: ToolSchema {
                name: "context_search".to_string(),
                aliases: None,
                description: "Search chunks in a decomposed context manifest by keywords. Returns matching chunks with their content. Use this to find relevant parts of large contexts.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "manifest_id": {
                            "type": "string",
                            "description": "The manifest ID from the context decomposition"
                        },
                        "keywords": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Keywords to search for in chunk content and hints"
                        }
                    },
                    "required": ["manifest_id", "keywords"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for ContextSearchTool {
    async fn execute(&self, params: Value) -> crate::Result<Value> {
        let manifest_id = params
            .get("manifest_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("manifest_id is required".to_string()))?;

        let keywords: Vec<String> = params
            .get("keywords")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError::InvalidParams("keywords array is required".to_string()))?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        if keywords.is_empty() {
            return Ok(error_response("No valid keywords provided"));
        }

        let keyword_refs: Vec<&str> = keywords.iter().map(|s| s.as_str()).collect();

        debug!(manifest_id, keywords = ?keywords, "Searching chunks");

        match self.ops.search(manifest_id, &keyword_refs).await {
            Ok(matches) => {
                let results: Vec<Value> = matches
                    .iter()
                    .map(|(idx, content)| {
                        json!({
                            "index": idx,
                            "content": content
                        })
                    })
                    .collect();

                info!(manifest_id, match_count = results.len(), "Search completed");

                Ok(success_response(json!({
                    "manifest_id": manifest_id,
                    "match_count": results.len(),
                    "matches": results
                })))
            }
            Err(e) => {
                warn!(manifest_id, error = %e, "Failed to search chunks");
                Ok(error_response(&e))
            }
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

/// Context decompose tool - decompose large text into chunks.
pub struct ContextDecomposeTool {
    ops: SharedRlmOps,
    schema: ToolSchema,
}

impl ContextDecomposeTool {
    /// Create a new context decompose tool.
    pub fn new(ops: SharedRlmOps) -> Self {
        Self {
            ops,
            schema: ToolSchema {
                name: "context_decompose".to_string(),
                aliases: None,
                description: "Decompose a large text context into searchable chunks. Returns a manifest ID and chunk previews that can be used with context_probe and context_search.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The large text content to decompose"
                        }
                    },
                    "required": ["content"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for ContextDecomposeTool {
    async fn execute(&self, params: Value) -> crate::Result<Value> {
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("content is required".to_string()))?;

        debug!(content_len = content.len(), "Decomposing context");

        match self.ops.decompose(content).await {
            Ok(result) => {
                let previews: Vec<Value> = result
                    .previews
                    .iter()
                    .take(20) // Limit previews in response
                    .map(|c| {
                        json!({
                            "index": c.index,
                            "tokens": c.tokens,
                            "preview": c.preview,
                            "hints": c.hints
                        })
                    })
                    .collect();

                info!(
                    manifest_id = %result.manifest_id,
                    chunk_count = result.chunk_count,
                    total_tokens = result.total_tokens,
                    "Context decomposed"
                );

                Ok(success_response(json!({
                    "manifest_id": result.manifest_id,
                    "chunk_count": result.chunk_count,
                    "total_tokens": result.total_tokens,
                    "chunk_previews": previews,
                    "message": format!(
                        "Context decomposed into {} chunks ({} tokens). Use context_probe(indices) to fetch specific chunks or context_search(keywords) to find relevant chunks.",
                        result.chunk_count,
                        result.total_tokens
                    )
                })))
            }
            Err(e) => {
                warn!(error = %e, "Failed to decompose context");
                Ok(error_response(&e))
            }
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

/// Create all RLM context tools with shared operations.
pub fn create_context_tools(ops: SharedRlmOps) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ContextDecomposeTool::new(Arc::clone(&ops))),
        Box::new(ContextProbeTool::new(Arc::clone(&ops))),
        Box::new(ContextSearchTool::new(ops)),
    ]
}

/// Get tool schemas without creating tools (for capability advertisement).
pub fn context_tool_schemas() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "context_decompose".to_string(),
            aliases: None,
            description: "Decompose a large text context into searchable chunks.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "content": { "type": "string" }
                },
                "required": ["content"]
            }),
        },
        ToolSchema {
            name: "context_probe".to_string(),
            aliases: None,
            description: "Fetch specific chunks from a decomposed context manifest.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "manifest_id": { "type": "string" },
                    "indices": { "type": "array", "items": { "type": "integer" } }
                },
                "required": ["manifest_id", "indices"]
            }),
        },
        ToolSchema {
            name: "context_search".to_string(),
            aliases: None,
            description: "Search chunks in a decomposed context manifest by keywords.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "manifest_id": { "type": "string" },
                    "keywords": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["manifest_id", "keywords"]
            }),
        },
    ]
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    struct MockRlmOps;

    #[async_trait]
    impl RlmOperations for MockRlmOps {
        async fn decompose(&self, _content: &str) -> Result<DecomposeResult, String> {
            Ok(DecomposeResult {
                manifest_id: "test-manifest".to_string(),
                chunk_count: 3,
                total_tokens: 150,
                previews: vec![ChunkPreview {
                    index: 0,
                    tokens: 50,
                    preview: "First chunk...".to_string(),
                    hints: vec!["test".to_string()],
                }],
            })
        }

        async fn probe(
            &self,
            _manifest_id: &str,
            indices: &[usize],
        ) -> Result<Vec<(usize, String)>, String> {
            Ok(indices
                .iter()
                .map(|&i| (i, format!("Content of chunk {i}")))
                .collect())
        }

        async fn search(
            &self,
            _manifest_id: &str,
            _keywords: &[&str],
        ) -> Result<Vec<(usize, String)>, String> {
            Ok(vec![(0, "Matching content".to_string())])
        }
    }

    #[tokio::test]
    async fn test_probe_tool() {
        let ops: SharedRlmOps = Arc::new(MockRlmOps);
        let tool = ContextProbeTool::new(ops);

        let result = tool
            .execute(json!({
                "manifest_id": "test",
                "indices": [0, 1]
            }))
            .await
            .unwrap();

        assert!(
            result
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
    }

    #[tokio::test]
    async fn test_search_tool() {
        let ops: SharedRlmOps = Arc::new(MockRlmOps);
        let tool = ContextSearchTool::new(ops);

        let result = tool
            .execute(json!({
                "manifest_id": "test",
                "keywords": ["auth"]
            }))
            .await
            .unwrap();

        assert!(
            result
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
    }

    #[tokio::test]
    async fn test_decompose_tool() {
        let ops: SharedRlmOps = Arc::new(MockRlmOps);
        let tool = ContextDecomposeTool::new(ops);

        let result = tool
            .execute(json!({
                "content": "Test content"
            }))
            .await
            .unwrap();

        assert!(
            result
                .get("success")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        );
        let data = result.get("data").unwrap();
        assert_eq!(data.get("manifest_id").unwrap(), "test-manifest");
    }

    #[test]
    fn test_tool_schemas() {
        let schemas = context_tool_schemas();
        assert_eq!(schemas.len(), 3);
        assert_eq!(schemas[0].name, "context_decompose");
        assert_eq!(schemas[1].name, "context_probe");
        assert_eq!(schemas[2].name, "context_search");
    }
}
