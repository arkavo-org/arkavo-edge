use crate::mcp::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use arkavo_memory::storage::MemoryStorage;
use arkavo_repo::repository_context::RepositoryContextManager;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

pub struct RepositoryContextTool {
    schema: ToolSchema,
    context_manager: RepositoryContextManager,
}

impl RepositoryContextTool {
    pub async fn new() -> Result<Self> {
        let memory_storage = Arc::new(
            MemoryStorage::new()
                .await
                .map_err(|e| TestError::Mcp(e.to_string()))?,
        );
        let context_manager = RepositoryContextManager::new(memory_storage)
            .map_err(|e| TestError::Mcp(e.to_string()))?;
        Ok(Self {
            schema: ToolSchema {
                name: "build_repository_context".to_string(),
                description: "Builds a comprehensive context of the current repository, including git status, recent commits, and file structure.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {},
                }),
            },
            context_manager,
        })
    }
}

#[async_trait]
impl Tool for RepositoryContextTool {
    async fn execute(&self, _params: Value) -> Result<Value> {
        let context = self
            .context_manager
            .build_context()
            .await
            .map_err(|e| TestError::Mcp(e.to_string()))?;
        Ok(serde_json::to_value(context)?)
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
