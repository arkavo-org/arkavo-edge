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

#[cfg(feature = "iroh-storage")]
pub use decomposer::IrohChunkStorage;
