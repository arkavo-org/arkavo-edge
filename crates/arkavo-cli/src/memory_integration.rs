#[cfg(all(target_os = "macos", feature = "test-harness"))]
use arkavo_mcp::Tool as McpTool;
#[cfg(all(target_os = "macos", feature = "test-harness"))]
use arkavo_mcp_macos::mcp::server::Tool;
#[cfg(all(target_os = "macos", feature = "test-harness"))]
use arkavo_memory::mcp_tools::{
    CategorizeMemoryTool, GetMemoryTool, SearchMemoryTool, StoreMemoryTool,
};
#[cfg(all(target_os = "macos", feature = "test-harness"))]
use arkavo_memory::storage::MemoryStorage;
#[cfg(all(target_os = "macos", feature = "test-harness"))]
use async_trait::async_trait;
#[cfg(all(target_os = "macos", feature = "test-harness"))]
use serde_json::Value;
#[cfg(all(target_os = "macos", feature = "test-harness"))]
use std::collections::HashMap;
#[cfg(all(target_os = "macos", feature = "test-harness"))]
use std::sync::Arc;

// Adapter that wraps an McpTool to use TestError
#[cfg(all(target_os = "macos", feature = "test-harness"))]
struct McpToolAdapter<T: McpTool> {
    inner: T,
}

#[cfg(all(target_os = "macos", feature = "test-harness"))]
impl<T: McpTool> McpToolAdapter<T> {
    const fn new(tool: T) -> Self {
        Self { inner: tool }
    }
}

#[cfg(all(target_os = "macos", feature = "test-harness"))]
#[async_trait]
impl<T: McpTool> Tool for McpToolAdapter<T> {
    async fn execute(&self, params: Value) -> arkavo_mcp_macos::Result<Value> {
        self.inner
            .execute(params)
            .await
            .map_err(|e| arkavo_mcp_macos::TestError::Mcp(e.to_string()))
    }

    fn schema(&self) -> &arkavo_mcp::ToolSchema {
        self.inner.schema()
    }
}

#[cfg(all(target_os = "macos", feature = "test-harness"))]
pub struct MemoryIntegration {
    storage: Arc<MemoryStorage>,
}

#[cfg(all(target_os = "macos", feature = "test-harness"))]
impl MemoryIntegration {
    pub async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        log::info!("Initializing memory integration...");

        log::info!("Using bundled AllMiniLML6V2 model for text embeddings");

        let storage = Arc::new(MemoryStorage::new().await?);
        log::info!(
            "Memory storage initialized at: {}",
            MemoryStorage::get_data_directory()?.display()
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
