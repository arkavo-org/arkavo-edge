use crate::{
    chunker::SemanticChunker, compressor::ContextCompressor, deduplicator::Deduplicator,
    metrics::CompressionMetrics, Result,
};
use std::time::Instant;

pub struct CompressionPipeline {
    chunker: SemanticChunker,
    deduplicator: Deduplicator,
    compressor: ContextCompressor,
}

impl CompressionPipeline {
    pub async fn new(model_name: String, model_path: Option<String>) -> Result<Self> {
        Ok(Self {
            chunker: SemanticChunker::default(),
            deduplicator: Deduplicator::default(),
            compressor: ContextCompressor::new(model_name, model_path).await?,
        })
    }

    pub async fn compress(
        &self,
        text: &str,
        target_reduction: f64,
    ) -> Result<(String, CompressionMetrics)> {
        let start_time = Instant::now();

        let original_tokens = Self::estimate_tokens(text);

        tracing::info!(
            "Starting compression pipeline: {} original tokens, target reduction: {:.1}%",
            original_tokens,
            target_reduction * 100.0
        );

        let chunks = self.chunker.chunk(text);
        tracing::debug!("Chunked text into {} chunks", chunks.len());

        let deduplicated = self.deduplicator.deduplicate(chunks);
        tracing::debug!("Deduplicated to {} unique chunks", deduplicated.len());

        let mut compressed_chunks = Vec::new();
        for (i, chunk) in deduplicated.iter().enumerate() {
            tracing::debug!("Compressing chunk {}/{}", i + 1, deduplicated.len());

            let target_ratio = 1.0 - target_reduction;
            let compressed = self.compressor.compress(chunk, target_ratio).await?;

            if !compressed.is_empty() {
                compressed_chunks.push(compressed);
            }
        }

        let compressed_text = compressed_chunks.join("\n\n");
        let compressed_tokens = Self::estimate_tokens(&compressed_text);

        let compression_time_ms = start_time.elapsed().as_millis() as u64;

        let metrics =
            CompressionMetrics::new(original_tokens, compressed_tokens, compression_time_ms);

        tracing::info!(
            "Compression complete: {} → {} tokens ({:.1}% reduction) in {}ms",
            original_tokens,
            compressed_tokens,
            metrics.reduction_percent,
            compression_time_ms
        );

        Ok((compressed_text, metrics))
    }

    fn estimate_tokens(text: &str) -> u32 {
        let words = text.split_whitespace().count();
        ((words as f64) / 0.75) as u32
    }

    pub fn compressor_model(&self) -> &str {
        self.compressor.model_name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pipeline_creation() {
        let result = CompressionPipeline::new(
            "gemma-3-270m-it".to_string(),
            Some("unsloth/gemma-3-270m-it-GGUF".to_string()),
        )
        .await;

        if result.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        assert!(result.is_ok());
    }

    #[test]
    fn test_token_estimation() {
        let text = "This is a test with ten words in the sentence";
        let tokens = CompressionPipeline::estimate_tokens(text);
        assert!(tokens > 0);
        assert!(tokens < 50);
    }

    #[tokio::test]
    async fn test_empty_text_compression() {
        let pipeline = CompressionPipeline::new(
            "gemma-3-270m-it".to_string(),
            Some("unsloth/gemma-3-270m-it-GGUF".to_string()),
        )
        .await;

        if pipeline.is_err() {
            eprintln!("Skipping test: Local model not available");
            return;
        }
        let pipeline = pipeline.unwrap();

        let result = pipeline.compress("", 0.5).await;
        assert!(result.is_ok());

        let (compressed, metrics) = result.unwrap();
        assert_eq!(compressed, "");
        assert_eq!(metrics.original_tokens, 0);
        assert_eq!(metrics.compressed_tokens, 0);
    }
}
