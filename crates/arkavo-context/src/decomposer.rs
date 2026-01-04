//! Context decomposition for RLM-style processing.
//!
//! Decomposes large contexts into content-addressed chunks that can be
//! stored externally (e.g., Iroh P2P) and queried on demand.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tiktoken_rs::cl100k_base;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument};
use uuid::Uuid;

use crate::chunker::SemanticChunker;
use crate::error::{Error, Result};

/// Reference to a stored chunk with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRef {
    /// Unique chunk identifier.
    pub id: String,
    /// Zero-based index in the original context.
    pub index: usize,
    /// Token count for this chunk.
    pub token_count: usize,
    /// Character count for this chunk.
    pub char_count: usize,
    /// Storage ticket (Iroh blob ticket or in-memory key).
    pub ticket: String,
    /// First 100 chars as preview/hint.
    pub preview: String,
    /// Semantic hints extracted from content (keywords).
    pub hints: Vec<String>,
}

/// Manifest describing a decomposed context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManifest {
    /// Unique manifest identifier.
    pub id: String,
    /// All chunk references in order.
    pub chunks: Vec<ChunkRef>,
    /// Total token count across all chunks.
    pub total_tokens: usize,
    /// Total character count.
    pub total_chars: usize,
    /// Number of chunks.
    pub chunk_count: usize,
    /// Chunk overlap in bytes (for reconstruction).
    pub overlap_bytes: usize,
    /// Created timestamp.
    pub created_at: u64,
}

impl ContextManifest {
    /// Get chunk indices matching any of the given keywords.
    pub fn search_hints(&self, keywords: &[&str]) -> Vec<usize> {
        self.chunks
            .iter()
            .enumerate()
            .filter(|(_, chunk)| {
                keywords.iter().any(|kw| {
                    let kw_lower = kw.to_lowercase();
                    chunk.hints.iter().any(|h| h.contains(&kw_lower))
                        || chunk.preview.to_lowercase().contains(&kw_lower)
                })
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Get a summary string for LLM context injection.
    pub fn to_summary(&self) -> String {
        format!(
            "Context decomposed into {} chunks ({} tokens total). \
             Use context_probe(indices) to fetch specific chunks. \
             Use context_search(keywords) to find relevant chunks.",
            self.chunk_count, self.total_tokens
        )
    }
}

/// Storage backend trait for chunk persistence.
#[async_trait::async_trait]
pub trait ChunkStorage: Send + Sync {
    /// Store a chunk and return its ticket/key.
    async fn store(&self, data: &[u8]) -> Result<String>;
    /// Fetch a chunk by its ticket/key.
    async fn fetch(&self, ticket: &str) -> Result<Vec<u8>>;
}

/// In-memory chunk storage (default, no external dependencies).
#[derive(Default)]
pub struct MemoryChunkStorage {
    chunks: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl MemoryChunkStorage {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl ChunkStorage for MemoryChunkStorage {
    async fn store(&self, data: &[u8]) -> Result<String> {
        let key = format!("mem:{}", Uuid::new_v4());
        self.chunks.write().await.insert(key.clone(), data.to_vec());
        Ok(key)
    }

    async fn fetch(&self, ticket: &str) -> Result<Vec<u8>> {
        self.chunks
            .read()
            .await
            .get(ticket)
            .cloned()
            .ok_or_else(|| Error::ChunkNotFound(ticket.to_string()))
    }
}

/// Iroh-based chunk storage (requires `iroh-storage` feature).
#[cfg(feature = "iroh-storage")]
pub struct IrohChunkStorage {
    transport: arkavo_tdf_iroh::IrohTransport,
}

#[cfg(feature = "iroh-storage")]
impl IrohChunkStorage {
    pub async fn new() -> std::result::Result<Self, arkavo_tdf_iroh::IrohError> {
        let transport = arkavo_tdf_iroh::IrohTransport::new().await?;
        Ok(Self { transport })
    }

    pub fn from_transport(transport: arkavo_tdf_iroh::IrohTransport) -> Self {
        Self { transport }
    }
}

#[cfg(feature = "iroh-storage")]
#[async_trait::async_trait]
impl ChunkStorage for IrohChunkStorage {
    async fn store(&self, data: &[u8]) -> Result<String> {
        use arkavo_tdf::BlobTransport;
        self.transport
            .stage(data)
            .await
            .map_err(|e| Error::StorageError(e.to_string()))
    }

    async fn fetch(&self, ticket: &str) -> Result<Vec<u8>> {
        use arkavo_tdf::BlobTransport;
        self.transport
            .fetch(ticket)
            .await
            .map_err(|e| Error::StorageError(e.to_string()))
    }
}

/// Context decomposer for RLM-style processing.
///
/// Splits large contexts into semantically coherent chunks,
/// stores them externally, and returns a manifest for querying.
pub struct ContextDecomposer<S: ChunkStorage> {
    chunker: SemanticChunker,
    storage: S,
}

impl ContextDecomposer<MemoryChunkStorage> {
    /// Create a new decomposer with in-memory storage.
    pub fn new() -> Self {
        Self {
            chunker: SemanticChunker::default(),
            storage: MemoryChunkStorage::new(),
        }
    }

    /// Create with custom chunk size (in bytes).
    pub fn with_chunk_size(max_chunk_size: usize, overlap_size: usize) -> Self {
        Self {
            chunker: SemanticChunker::new(max_chunk_size, overlap_size),
            storage: MemoryChunkStorage::new(),
        }
    }
}

impl Default for ContextDecomposer<MemoryChunkStorage> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: ChunkStorage> ContextDecomposer<S> {
    /// Create with custom storage backend.
    pub fn with_storage(storage: S) -> Self {
        Self {
            chunker: SemanticChunker::default(),
            storage,
        }
    }

    /// Create with custom chunker and storage.
    pub fn with_chunker_and_storage(chunker: SemanticChunker, storage: S) -> Self {
        Self { chunker, storage }
    }

    /// Decompose a large context into stored chunks.
    #[instrument(skip(self, context), fields(context_len = context.len()))]
    pub async fn decompose(&self, context: &str) -> Result<ContextManifest> {
        let bpe = cl100k_base().map_err(|e| Error::TokenizerError(e.to_string()))?;

        // Split into chunks
        let chunks = self.chunker.chunk(context);
        info!(chunk_count = chunks.len(), "Context chunked");

        let mut chunk_refs = Vec::with_capacity(chunks.len());
        let mut total_tokens = 0;
        let mut total_chars = 0;

        for (index, chunk_content) in chunks.iter().enumerate() {
            // Calculate token count
            let tokens = bpe.encode_ordinary(chunk_content);
            let token_count = tokens.len();
            total_tokens += token_count;
            total_chars += chunk_content.len();

            // Store chunk
            let ticket = self.storage.store(chunk_content.as_bytes()).await?;
            debug!(index, token_count, ticket = %ticket, "Chunk stored");

            // Extract hints (simple keyword extraction)
            let hints = extract_hints(chunk_content);

            // Create preview
            let preview = if chunk_content.len() > 100 {
                format!("{}...", &chunk_content[..100])
            } else {
                chunk_content.clone()
            };

            chunk_refs.push(ChunkRef {
                id: Uuid::new_v4().to_string(),
                index,
                token_count,
                char_count: chunk_content.len(),
                ticket,
                preview,
                hints,
            });
        }

        let manifest = ContextManifest {
            id: Uuid::new_v4().to_string(),
            chunks: chunk_refs,
            total_tokens,
            total_chars,
            chunk_count: chunks.len(),
            overlap_bytes: 200, // Default overlap from SemanticChunker
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };

        info!(
            manifest_id = %manifest.id,
            total_tokens,
            chunk_count = manifest.chunk_count,
            "Context decomposed"
        );

        Ok(manifest)
    }

    /// Fetch specific chunks by indices.
    #[instrument(skip(self, manifest))]
    pub async fn probe(&self, manifest: &ContextManifest, indices: &[usize]) -> Result<Vec<String>> {
        let mut results = Vec::with_capacity(indices.len());

        for &idx in indices {
            if idx >= manifest.chunks.len() {
                return Err(Error::ChunkNotFound(format!("Index {} out of bounds", idx)));
            }

            let chunk_ref = &manifest.chunks[idx];
            let data = self.storage.fetch(&chunk_ref.ticket).await?;
            let content = String::from_utf8(data)
                .map_err(|e| Error::StorageError(format!("Invalid UTF-8: {}", e)))?;
            results.push(content);
        }

        debug!(indices = ?indices, fetched = results.len(), "Chunks probed");
        Ok(results)
    }

    /// Search chunks by keywords and return matching content.
    #[instrument(skip(self, manifest))]
    pub async fn search(
        &self,
        manifest: &ContextManifest,
        keywords: &[&str],
    ) -> Result<Vec<(usize, String)>> {
        let matching_indices = manifest.search_hints(keywords);
        debug!(keywords = ?keywords, matches = matching_indices.len(), "Search complete");

        let mut results = Vec::with_capacity(matching_indices.len());
        for idx in matching_indices {
            let chunk_ref = &manifest.chunks[idx];
            let data = self.storage.fetch(&chunk_ref.ticket).await?;
            let content = String::from_utf8(data)
                .map_err(|e| Error::StorageError(format!("Invalid UTF-8: {}", e)))?;
            results.push((idx, content));
        }

        Ok(results)
    }
}

/// Extract simple keyword hints from chunk content.
fn extract_hints(content: &str) -> Vec<String> {
    let mut hints = Vec::new();

    // Extract potential identifiers (function names, classes, etc.)
    let words: Vec<&str> = content
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 4 && w.len() <= 50)
        .collect();

    // Keep unique lowercase words as hints (limit to 20)
    let mut seen = std::collections::HashSet::new();
    for word in words {
        let lower = word.to_lowercase();
        if seen.insert(lower.clone()) {
            hints.push(lower);
            if hints.len() >= 20 {
                break;
            }
        }
    }

    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_decompose_small_context() {
        let decomposer = ContextDecomposer::new();
        let context = "This is a small test context.";

        let manifest = decomposer.decompose(context).await.unwrap();

        assert_eq!(manifest.chunk_count, 1);
        assert!(manifest.total_tokens > 0);
        assert_eq!(manifest.total_chars, context.len());
    }

    #[tokio::test]
    async fn test_decompose_large_context() {
        let decomposer = ContextDecomposer::with_chunk_size(100, 20);

        // Create a context larger than chunk size with paragraph breaks
        let paragraph = "This is a test paragraph with some content.";
        let context = (0..50)
            .map(|i| format!("Paragraph {} - {}", i, paragraph))
            .collect::<Vec<_>>()
            .join("\n\n");

        let manifest = decomposer.decompose(&context).await.unwrap();

        assert!(manifest.chunk_count > 1, "Expected multiple chunks, got {}", manifest.chunk_count);
        assert!(manifest.total_chars > 100);
    }

    #[tokio::test]
    async fn test_probe_chunks() {
        let decomposer = ContextDecomposer::with_chunk_size(100, 20);
        let context = "First chunk content.\n\nSecond chunk with more text.\n\nThird chunk here.";

        let manifest = decomposer.decompose(context).await.unwrap();
        let probed = decomposer.probe(&manifest, &[0]).await.unwrap();

        assert_eq!(probed.len(), 1);
        assert!(probed[0].contains("First") || probed[0].contains("chunk"));
    }

    #[tokio::test]
    async fn test_search_chunks() {
        let decomposer = ContextDecomposer::with_chunk_size(200, 20);
        let context = "Function authentication handles login.\n\n\
                       Database connection pool setup.\n\n\
                       Authentication token validation.";

        let manifest = decomposer.decompose(context).await.unwrap();
        let results = decomposer.search(&manifest, &["authentication"]).await.unwrap();

        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_manifest_summary() {
        let decomposer = ContextDecomposer::new();
        let context = "Test context for summary generation.";

        let manifest = decomposer.decompose(context).await.unwrap();
        let summary = manifest.to_summary();

        assert!(summary.contains("1 chunks"));
        assert!(summary.contains("context_probe"));
    }
}
