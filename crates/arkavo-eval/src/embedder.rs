//! Embedder implementations. `MemoryEmbedder` is the production semantic
//! embedder (arkavo-memory's bundled offline ONNX model); `CharEmbedder` is a
//! deterministic fallback used when the ONNX model files are not present.

use crate::verdict::{Embedder, VerdictError};
use async_trait::async_trait;

/// Deterministic char-frequency embedder (no external model).
pub struct CharEmbedder;

#[async_trait]
impl Embedder for CharEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
        let mut v = vec![0.0f32; 27];
        for c in text.to_lowercase().chars() {
            if c.is_ascii_lowercase() {
                v[(c as u8 - b'a') as usize] += 1.0;
            } else {
                v[26] += 1.0;
            }
        }
        Ok(v)
    }
}

#[cfg(feature = "embeddings")]
pub struct MemoryEmbedder {
    inner: arkavo_memory::EmbeddingService,
}

#[cfg(feature = "embeddings")]
impl MemoryEmbedder {
    pub fn new() -> Self {
        Self {
            inner: arkavo_memory::EmbeddingService::new(),
        }
    }

    /// True if the bundled ONNX model files are loadable in this process.
    pub async fn available(&self) -> bool {
        self.inner.ensure_model_available().await.is_ok()
    }
}

#[cfg(feature = "embeddings")]
impl Default for MemoryEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "embeddings")]
#[async_trait]
impl Embedder for MemoryEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
        self.inner
            .generate_embedding(text)
            .await
            .map_err(|e| VerdictError::Embedding(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn char_embedder_is_deterministic() {
        let a = CharEmbedder.embed("paris").await.unwrap();
        let b = CharEmbedder.embed("paris").await.unwrap();
        assert_eq!(a, b);
        assert_eq!(crate::verdict::cosine(&a, &b), 1.0);
    }
}
