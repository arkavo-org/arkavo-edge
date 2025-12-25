use crate::server::Tool;
use arkavo_mcp::ToolSchema;
use arkavo_memory::{ContextLedger, MemoryStorage};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct ContextRestoreTool {
    schema: ToolSchema,
    storage: Arc<MemoryStorage>,
}

impl ContextRestoreTool {
    pub fn new(storage: Arc<MemoryStorage>) -> Self {
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
        Self { schema, storage }
    }
}

#[async_trait]
impl Tool for ContextRestoreTool {
    async fn execute(&self, params: Value) -> crate::Result<Value> {
        let id_str = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::ToolError::InvalidParams("Missing 'id'".to_string()))?;

        let ledger = ContextLedger::new(self.storage.clone());

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
