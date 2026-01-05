//! RLM Bridge - connects MCP context tools to arkavo-router's RlmContextManager.
//!
//! Implements the RlmOperations trait to enable context_probe, context_search,
//! and context_decompose MCP tools to work with the RLM system.

use arkavo_mcp_tools::context_tools::{ChunkPreview, DecomposeResult, RlmOperations};
use arkavo_router::rlm::SharedRlmManager;
use async_trait::async_trait;
use tracing::{debug, info};

/// Bridge between MCP tools and RLM manager.
pub struct RlmBridge {
    manager: SharedRlmManager,
}

impl RlmBridge {
    /// Create a new RLM bridge with the given manager.
    pub fn new(manager: SharedRlmManager) -> Self {
        Self { manager }
    }

    /// Create with a new default manager.
    pub fn with_default_manager() -> Self {
        Self {
            manager: arkavo_router::rlm::create_rlm_manager(),
        }
    }

    /// Get the underlying manager reference.
    pub fn manager(&self) -> &SharedRlmManager {
        &self.manager
    }

    /// Check if RLM mode should activate for given token count.
    pub fn should_activate(&self, input_tokens: usize, model_context_size: usize) -> bool {
        self.manager.should_activate(input_tokens, model_context_size)
    }

    /// Generate system prompt for RLM mode.
    pub fn generate_system_prompt(
        &self,
        result: &arkavo_router::rlm::RlmDecompositionResult,
    ) -> String {
        self.manager.generate_rlm_system_prompt(result)
    }
}

#[async_trait]
impl RlmOperations for RlmBridge {
    async fn decompose(&self, content: &str) -> Result<DecomposeResult, String> {
        info!(content_len = content.len(), "RLM bridge: decomposing context");

        let result = self
            .manager
            .decompose(content)
            .await
            .map_err(|e| e.to_string())?;

        let previews: Vec<ChunkPreview> = result
            .chunk_previews
            .iter()
            .map(|p| ChunkPreview {
                index: p.index,
                tokens: p.tokens,
                preview: p.preview.clone(),
                hints: p.hints.clone(),
            })
            .collect();

        debug!(
            manifest_id = %result.manifest_id,
            chunk_count = result.chunk_count,
            "RLM bridge: decomposition complete"
        );

        Ok(DecomposeResult {
            manifest_id: result.manifest_id,
            chunk_count: result.chunk_count,
            total_tokens: result.total_tokens,
            previews,
        })
    }

    async fn probe(
        &self,
        manifest_id: &str,
        indices: &[usize],
    ) -> Result<Vec<(usize, String)>, String> {
        debug!(manifest_id, indices = ?indices, "RLM bridge: probing chunks");

        let result = self
            .manager
            .probe(manifest_id, indices)
            .await
            .map_err(|e| e.to_string())?;

        Ok(result
            .chunks
            .into_iter()
            .map(|c| (c.index, c.content))
            .collect())
    }

    async fn search(
        &self,
        manifest_id: &str,
        keywords: &[&str],
    ) -> Result<Vec<(usize, String)>, String> {
        debug!(manifest_id, keywords = ?keywords, "RLM bridge: searching chunks");

        let result = self
            .manager
            .search(manifest_id, keywords)
            .await
            .map_err(|e| e.to_string())?;

        Ok(result
            .matches
            .into_iter()
            .map(|m| (m.index, m.content))
            .collect())
    }
}

/// Estimate token count for text (rough approximation).
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

/// Get default model context size based on model hint.
pub fn model_context_size(model_hint: Option<&str>, is_cloud: bool) -> usize {
    if is_cloud {
        131_072 // 128K for cloud models
    } else {
        match model_hint {
            Some("270M") => 2_048,
            Some("1B") => 4_096,
            Some("2B") | Some("3B") => 8_192,
            Some("7B") | Some("8B") => 32_768,
            Some("14B") => 32_768,
            _ => 8_192, // Default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("hello"), 1);
        assert_eq!(estimate_tokens("hello world test"), 4);
    }

    #[test]
    fn test_model_context_size() {
        assert_eq!(model_context_size(Some("270M"), false), 2_048);
        assert_eq!(model_context_size(Some("7B"), false), 32_768);
        assert_eq!(model_context_size(None, true), 131_072);
    }

    #[tokio::test]
    async fn test_rlm_bridge_creation() {
        let bridge = RlmBridge::with_default_manager();
        assert!(!bridge.should_activate(1000, 8192));
        assert!(bridge.should_activate(6000, 8192)); // 70% of 8192
    }
}
