#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unused_async)]

pub mod chunker;
pub mod compressor;
pub mod deduplicator;
pub mod error;
pub mod metrics;
pub mod pipeline;

pub use chunker::SemanticChunker;
pub use compressor::ContextCompressor;
pub use deduplicator::Deduplicator;
pub use error::{Error, Result};
pub use metrics::{CompressionMetrics, CompressionStats};
pub use pipeline::CompressionPipeline;
