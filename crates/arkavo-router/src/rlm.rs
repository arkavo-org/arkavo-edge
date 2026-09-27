//! Recursive Language Model (RLM) routing for handling large contexts.
//!
//! Implements RLM-style processing where large contexts are decomposed into
//! content-addressed chunks that can be probed, searched, and processed
//! by sub-agents in an agent mesh.
//!
//! Based on the RLM paper (arXiv:2512.24601v1) which treats prompts as
//! external environment variables rather than feeding them directly to models.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

use arkavo_context::{ChunkRef, ContextDecomposer, ContextManifest, MemoryChunkStorage};

use crate::error::{Error, Result};

/// Configuration for RLM routing.
#[derive(Debug, Clone)]
pub struct RlmConfig {
    /// Token threshold above which RLM mode is activated (default: 70% of model context).
    pub activation_threshold_ratio: f32,
    /// Maximum recursion depth for sub-queries (default: 1).
    pub max_depth: usize,
    /// Maximum number of chunks to probe in a single operation.
    pub max_probe_chunks: usize,
    /// Whether to enable parallel chunk processing.
    pub parallel_processing: bool,
}

impl Default for RlmConfig {
    fn default() -> Self {
        Self {
            activation_threshold_ratio: 0.7,
            max_depth: 1,
            max_probe_chunks: 10,
            parallel_processing: true,
        }
    }
}

/// Result from RLM context decomposition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmDecompositionResult {
    /// The manifest ID for future operations.
    pub manifest_id: String,
    /// Number of chunks created.
    pub chunk_count: usize,
    /// Total tokens in original context.
    pub total_tokens: usize,
    /// Summary for LLM context injection.
    pub summary: String,
    /// Chunk previews with hints for search.
    pub chunk_previews: Vec<ChunkPreview>,
}

/// Preview of a chunk for LLM context injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkPreview {
    pub index: usize,
    pub tokens: usize,
    pub preview: String,
    pub hints: Vec<String>,
}

impl From<&ChunkRef> for ChunkPreview {
    fn from(chunk: &ChunkRef) -> Self {
        Self {
            index: chunk.index,
            tokens: chunk.token_count,
            preview: chunk.preview.clone(),
            hints: chunk.hints.clone(),
        }
    }
}

/// Result from probing chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmProbeResult {
    pub chunks: Vec<ProbeChunk>,
}

/// A probed chunk with its content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeChunk {
    pub index: usize,
    pub content: String,
}

/// Result from searching chunks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmSearchResult {
    pub match_count: usize,
    pub matches: Vec<SearchMatch>,
}

/// A search match with chunk content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    pub index: usize,
    pub content: String,
}

/// RLM context manager for handling large inputs.
///
/// Provides decomposition, probing, and search operations for contexts
/// that exceed model context windows.
pub struct RlmContextManager {
    decomposer: ContextDecomposer<MemoryChunkStorage>,
    manifests: RwLock<HashMap<String, ContextManifest>>,
    config: RlmConfig,
}

impl RlmContextManager {
    /// Create a new RLM context manager with default configuration.
    pub fn new() -> Self {
        Self::with_config(RlmConfig::default())
    }

    /// Create with custom configuration.
    pub fn with_config(config: RlmConfig) -> Self {
        Self {
            decomposer: ContextDecomposer::new(),
            manifests: RwLock::new(HashMap::new()),
            config,
        }
    }

    /// Get the configuration.
    pub fn config(&self) -> &RlmConfig {
        &self.config
    }

    /// Check if RLM mode should be activated based on input size.
    pub fn should_activate(&self, input_tokens: usize, model_context_size: usize) -> bool {
        let threshold =
            (model_context_size as f32 * self.config.activation_threshold_ratio) as usize;
        input_tokens > threshold
    }

    /// Decompose a large context into chunks.
    #[instrument(skip(self, context), fields(context_len = context.len()))]
    pub async fn decompose(&self, context: &str) -> Result<RlmDecompositionResult> {
        let manifest = self
            .decomposer
            .decompose(context)
            .await
            .map_err(|e| Error::ContextDecomposition(e.to_string()))?;

        let manifest_id = manifest.id.clone();
        let chunk_count = manifest.chunk_count;
        let total_tokens = manifest.total_tokens;
        let summary = manifest.to_summary();
        let chunk_previews: Vec<ChunkPreview> =
            manifest.chunks.iter().map(ChunkPreview::from).collect();

        // Store manifest for future operations
        self.manifests
            .write()
            .await
            .insert(manifest_id.clone(), manifest);

        info!(
            manifest_id = %manifest_id,
            chunk_count,
            total_tokens,
            "Context decomposed for RLM processing"
        );

        Ok(RlmDecompositionResult {
            manifest_id,
            chunk_count,
            total_tokens,
            summary,
            chunk_previews,
        })
    }

    /// Get a manifest by ID.
    pub async fn get_manifest(&self, id: &str) -> Option<ContextManifest> {
        self.manifests.read().await.get(id).cloned()
    }

    /// Probe specific chunks by indices.
    #[instrument(skip(self))]
    pub async fn probe(&self, manifest_id: &str, indices: &[usize]) -> Result<RlmProbeResult> {
        // Limit number of chunks
        if indices.len() > self.config.max_probe_chunks {
            warn!(
                requested = indices.len(),
                max = self.config.max_probe_chunks,
                "Truncating probe request to max chunks"
            );
        }
        let indices: Vec<usize> = indices
            .iter()
            .take(self.config.max_probe_chunks)
            .copied()
            .collect();

        let manifest = self
            .get_manifest(manifest_id)
            .await
            .ok_or_else(|| Error::ManifestNotFound(manifest_id.to_string()))?;

        let contents = self
            .decomposer
            .probe(&manifest, &indices)
            .await
            .map_err(|e| Error::ContextDecomposition(e.to_string()))?;

        let chunks: Vec<ProbeChunk> = indices
            .iter()
            .zip(contents.iter())
            .map(|(&index, content)| ProbeChunk {
                index,
                content: content.clone(),
            })
            .collect();

        debug!(manifest_id, chunk_count = chunks.len(), "Chunks probed");

        Ok(RlmProbeResult { chunks })
    }

    /// Search chunks by keywords.
    #[instrument(skip(self))]
    pub async fn search(&self, manifest_id: &str, keywords: &[&str]) -> Result<RlmSearchResult> {
        let manifest = self
            .get_manifest(manifest_id)
            .await
            .ok_or_else(|| Error::ManifestNotFound(manifest_id.to_string()))?;

        let results = self
            .decomposer
            .search(&manifest, keywords)
            .await
            .map_err(|e| Error::ContextDecomposition(e.to_string()))?;

        let matches: Vec<SearchMatch> = results
            .into_iter()
            .map(|(index, content)| SearchMatch { index, content })
            .collect();

        debug!(
            manifest_id,
            keywords = ?keywords,
            match_count = matches.len(),
            "Chunks searched"
        );

        Ok(RlmSearchResult {
            match_count: matches.len(),
            matches,
        })
    }

    /// Generate a system prompt with manifest reference for RLM-style processing.
    pub fn generate_rlm_system_prompt(&self, result: &RlmDecompositionResult) -> String {
        let chunk_list: String = result
            .chunk_previews
            .iter()
            .take(20) // Limit to first 20 chunks in prompt
            .map(|c| {
                format!(
                    "  [{}] {} ({}tok): {}",
                    c.index,
                    c.hints.join(", "),
                    c.tokens,
                    c.preview
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"Context has {chunk_count} chunks. Use tools to access code.

Chunks: {chunk_list}

Tools available:
- context_search: find code by keywords
- context_probe: get chunk content by index

Call tools using this exact format:
```context_search
manifest_id: {manifest_id}
keywords: your, search, terms
```

```context_probe
manifest_id: {manifest_id}
indices: 0, 1, 2
```"#,
            chunk_count = result.chunk_count,
            chunk_list = chunk_list,
            manifest_id = result.manifest_id,
        )
    }

    /// Clear a manifest from memory.
    pub async fn clear_manifest(&self, manifest_id: &str) -> bool {
        self.manifests.write().await.remove(manifest_id).is_some()
    }

    /// Get statistics about stored manifests.
    pub async fn stats(&self) -> RlmStats {
        let manifests = self.manifests.read().await;
        let total_chunks: usize = manifests.values().map(|m| m.chunk_count).sum();
        let total_tokens: usize = manifests.values().map(|m| m.total_tokens).sum();

        RlmStats {
            manifest_count: manifests.len(),
            total_chunks,
            total_tokens,
        }
    }
}

impl Default for RlmContextManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about RLM context storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmStats {
    pub manifest_count: usize,
    pub total_chunks: usize,
    pub total_tokens: usize,
}

/// Thread-safe shared RLM context manager.
pub type SharedRlmManager = Arc<RlmContextManager>;

/// Create a shared RLM context manager.
pub fn create_rlm_manager() -> SharedRlmManager {
    Arc::new(RlmContextManager::new())
}

/// Create a shared RLM context manager with custom config.
pub fn create_rlm_manager_with_config(config: RlmConfig) -> SharedRlmManager {
    Arc::new(RlmContextManager::with_config(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("ROUTER-009")]
    #[tokio::test]
    async fn test_rlm_decompose() {
        let manager = RlmContextManager::new();

        let context = "This is a test context.\n\nWith multiple paragraphs.\n\nFor RLM testing.";
        let result = manager.decompose(context).await.unwrap();

        assert!(!result.manifest_id.is_empty());
        assert!(result.chunk_count >= 1);
        assert!(result.total_tokens > 0);
    }

    #[spec("ROUTER-009")]
    #[tokio::test]
    async fn test_rlm_probe() {
        let manager = RlmContextManager::new();

        let context = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let decomp = manager.decompose(context).await.unwrap();

        let result = manager.probe(&decomp.manifest_id, &[0]).await.unwrap();

        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].index, 0);
    }

    #[spec("ROUTER-009")]
    #[tokio::test]
    async fn test_rlm_search() {
        let manager = RlmContextManager::new();

        let context =
            "Authentication module.\n\nDatabase connection.\n\nAuthentication validation.";
        let decomp = manager.decompose(context).await.unwrap();

        let result = manager
            .search(&decomp.manifest_id, &["authentication"])
            .await
            .unwrap();

        assert!(result.match_count > 0);
    }

    #[spec("ROUTER-009")]
    #[tokio::test]
    async fn test_should_activate() {
        let manager = RlmContextManager::new();

        // 7000 tokens with 10000 model context at 70% threshold = 7000
        assert!(!manager.should_activate(6000, 10000));
        assert!(manager.should_activate(8000, 10000));
    }

    #[spec("ROUTER-009")]
    #[tokio::test]
    async fn test_system_prompt_generation() {
        let manager = RlmContextManager::new();

        let context = "Test content for prompt generation.";
        let result = manager.decompose(context).await.unwrap();

        let prompt = manager.generate_rlm_system_prompt(&result);

        assert!(prompt.contains("context_probe"));
        assert!(prompt.contains("context_search"));
        assert!(prompt.contains(&result.manifest_id));
    }
}
