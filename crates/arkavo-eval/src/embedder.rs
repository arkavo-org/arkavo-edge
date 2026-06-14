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

/// Discriminating lexical embedder: hashes word tokens AND character trigrams
/// into a fixed-width count vector, so cosine similarity reflects shared
/// words/sub-words rather than just character frequency. Deterministic,
/// pure-Rust, no external model. Used as the default eval embedder; the ONNX
/// `MemoryEmbedder` is the opt-in semantic upgrade.
pub struct LexicalEmbedder {
    dim: usize,
}

impl LexicalEmbedder {
    pub fn new() -> Self {
        Self { dim: 512 }
    }

    fn hash_feature(s: &str) -> u64 {
        // FNV-1a, deterministic and stable.
        let mut h: u64 = 0xcbf29ce484222325;
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
}

impl Default for LexicalEmbedder {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Embedder for LexicalEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, VerdictError> {
        let mut v = vec![0.0f32; self.dim];
        let lower = text.to_lowercase();
        // Word tokens (alphanumeric runs).
        for tok in lower.split(|c: char| !c.is_alphanumeric()) {
            if tok.is_empty() {
                continue;
            }
            let idx = (Self::hash_feature(&format!("w:{tok}")) as usize) % self.dim;
            v[idx] += 1.0;
        }
        // Character trigrams over the lowercased char sequence.
        let chars: Vec<char> = lower.chars().collect();
        for w in chars.windows(3) {
            let tri: String = w.iter().collect();
            let idx = (Self::hash_feature(&format!("t:{tri}")) as usize) % self.dim;
            v[idx] += 1.0;
        }
        Ok(v)
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

    #[tokio::test]
    async fn lexical_embedder_discriminates() {
        let e = LexicalEmbedder::new();
        let same = crate::verdict::cosine(
            &e.embed("Paris").await.unwrap(),
            &e.embed("Paris").await.unwrap(),
        );
        assert!((same - 1.0).abs() < 1e-6, "identical text -> cosine 1.0");
        let diff = crate::verdict::cosine(
            &e.embed("Paris").await.unwrap(),
            &e.embed("London").await.unwrap(),
        );
        assert!(
            diff < 0.5,
            "different words must be clearly dissimilar, got {diff}"
        );
        // The char-frequency embedder would rate these as quite similar; the
        // lexical one must not.
        let verbose = crate::verdict::cosine(
            &e.embed("70 km/h").await.unwrap(),
            &e.embed("Banana split with extra sprinkles").await.unwrap(),
        );
        assert!(
            verbose < 0.3,
            "unrelated answers must be dissimilar, got {verbose}"
        );
    }
}
