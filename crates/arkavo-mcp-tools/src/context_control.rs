use crate::server::Tool;
use arkavo_mcp::ToolSchema;
use arkavo_memory::{ContextLedger, HnswConfig, MemoryStorage};
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;

pub struct ContextRestoreTool {
    schema: ToolSchema,
    db_path: Option<PathBuf>,
}

impl Default for ContextRestoreTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextRestoreTool {
    pub fn new() -> Self {
        Self::with_path(None)
    }

    /// Creates a tool that uses a specific database path.
    /// Use this in tests or when sharing storage with a Conductor.
    pub fn with_path(db_path: Option<PathBuf>) -> Self {
        let schema = ToolSchema {
            name: "context_restore".to_string(),
            aliases: Some(vec!["restore_context".to_string()]),
            description: "Restores archived context fragments by ID. Use this when you see a [ARCHIVED: ... - ID: ...] pointer and need the details.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The UUID of the archived fragment to restore"
                    }
                },
                "required": ["id"]
            }),
        };
        Self { schema, db_path }
    }
}

#[async_trait]
impl Tool for ContextRestoreTool {
    async fn execute(&self, params: Value) -> crate::Result<Value> {
        let id_str = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::ToolError::InvalidParams("Missing 'id'".to_string()))?;

        // Use configured path if provided, otherwise use default
        let storage = match &self.db_path {
            Some(path) => MemoryStorage::with_path(path.clone(), HnswConfig::default()).await,
            None => MemoryStorage::new().await,
        }
        .map_err(|e| crate::ToolError::Other(e.to_string()))?;

        let ledger = ContextLedger::new(storage);

        let content = ledger
            .restore(id_str)
            .await
            .map_err(|e| crate::ToolError::Other(e.to_string()))?;

        Ok(serde_json::json!({ "content": content }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
