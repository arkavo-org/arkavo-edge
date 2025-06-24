use arkavo_memory::mcp_tools::{
    CategorizeMemoryTool, GetMemoryTool, SearchMemoryTool, StoreMemoryTool,
};
use arkavo_memory::storage::MemoryStorage;
use arkavo_mcp::Tool as McpTool;
use arkavo_test::mcp::server::Tool;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

// Adapter that wraps an McpTool to use TestError
struct McpToolAdapter<T: McpTool> {
    inner: T,
}

impl<T: McpTool> McpToolAdapter<T> {
    fn new(tool: T) -> Self {
        Self { inner: tool }
    }
}

#[async_trait]
impl<T: McpTool> Tool for McpToolAdapter<T> {
    async fn execute(&self, params: Value) -> arkavo_test::Result<Value> {
        self.inner
            .execute(params)
            .await
            .map_err(|e| arkavo_test::TestError::Mcp(e.to_string()))
    }

    fn schema(&self) -> &arkavo_mcp::ToolSchema {
        self.inner.schema()
    }
}

pub struct MemoryIntegration {
    storage: Arc<MemoryStorage>,
}

impl MemoryIntegration {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        log::info!("Initializing memory integration...");

        // Check Ollama availability
        let embedding_service = arkavo_memory::embeddings::EmbeddingService::new();
        if let Err(e) = embedding_service.ensure_model_available().await {
            log::warn!("Ollama embedding model not available: {}", e);
            log::info!("Memory features will be limited. To enable full functionality:");
            log::info!("  1. Install Ollama from https://ollama.ai");
            log::info!("  2. Run: ollama pull nomic-embed-text");
        } else {
            log::info!("Ollama is ready with embedding model");
        }

        let storage = Arc::new(MemoryStorage::new().await?);
        log::info!(
            "Memory storage initialized at: {:?}",
            MemoryStorage::get_data_directory()?
        );

        Ok(Self { storage })
    }

    pub fn get_tools(&self) -> HashMap<String, Box<dyn Tool>> {
        let mut tools: HashMap<String, Box<dyn Tool>> = HashMap::new();

        tools.insert(
            "store_memory".to_string(),
            Box::new(McpToolAdapter::new(StoreMemoryTool::new(
                self.storage.clone(),
            ))),
        );

        tools.insert(
            "search_memory".to_string(),
            Box::new(McpToolAdapter::new(SearchMemoryTool::new(
                self.storage.clone(),
            ))),
        );

        tools.insert(
            "get_memory".to_string(),
            Box::new(McpToolAdapter::new(GetMemoryTool::new(
                self.storage.clone(),
            ))),
        );

        tools.insert(
            "categorize_memory".to_string(),
            Box::new(McpToolAdapter::new(CategorizeMemoryTool::new(
                self.storage.clone(),
            ))),
        );

        tools
    }
}
