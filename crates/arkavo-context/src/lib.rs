#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unused_async)]

pub mod chunker;
pub mod compressor;
pub mod decomposer;
pub mod deduplicator;
pub mod error;
pub mod metrics;
pub mod pipeline;
pub mod prompt_enricher;
pub mod repo_context;
pub mod rlm_detection;
pub mod summarizer;

pub use chunker::SemanticChunker;
pub use compressor::ContextCompressor;
pub use decomposer::{
    ChunkRef, ChunkStorage, ContextDecomposer, ContextManifest, MemoryChunkStorage,
};
pub use deduplicator::Deduplicator;
pub use error::{Error, Result};
pub use metrics::{CompressionMetrics, CompressionStats};
pub use pipeline::CompressionPipeline;
pub use prompt_enricher::{
    CodeContext, FileContext, ProblemStatement, PromptEnricher, PromptTemplate,
};
pub use summarizer::ContextSummarizer;

// Re-export RLM detection types
pub use rlm_detection::{
    check_rlm_activation, context_usage_percentage, estimate_tokens, model_context_size,
};

// Re-export repo context types
pub use repo_context::{RepoContextMode, compress_repo_context, should_attach_repo_context};

#[cfg(feature = "iroh-storage")]
pub use decomposer::IrohChunkStorage;
