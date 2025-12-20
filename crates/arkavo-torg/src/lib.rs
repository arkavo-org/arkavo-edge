//! TØR-G constrained decoding integration for arkavo-edge
//!
//! This crate bridges `torg-core` and `torg-mask` with `arkavo-llama-cpp`
//! to enable constrained LLM decoding of boolean policy graphs.
//!
//! # Architecture
//!
//! ```text
//! User Request → Qwen3-0.6B → torg-mask → Graph → evaluate()
//!                (constrained   (logit    (torg-    (<1μs)
//!                 decoding)     masking)   core)
//! ```
//!
//! # Example
//!
//! ```ignore
//! use arkavo_torg::{Qwen3TokenMap, TorgLlamaSampler};
//!
//! // Build token mapping from model vocabulary
//! let mapping = Qwen3TokenMap::from_vocab(vocab)?;
//!
//! // Create constrained sampler
//! let mut sampler = TorgLlamaSampler::new(mapping.into_mapping(), vocab_size);
//!
//! // During inference, get logit bias for current state
//! let biases = sampler.get_logit_bias();
//! llama_sampler.add_logit_bias(vocab_size, &biases);
//!
//! // After sampling, feed token back
//! sampler.feed_token(sampled_token)?;
//!
//! // When complete, extract the graph
//! let graph = sampler.finish()?;
//! let result = graph.evaluate(&inputs);
//! ```

mod ministral;
mod prompt;
mod qwen3;
mod sampler;

pub use ministral::MinistralTokenMap;
pub use prompt::{TORG_SYSTEM_PROMPT, format_prompt};
pub use qwen3::Qwen3TokenMap;
pub use sampler::TorgLlamaSampler;

// Re-export core types for convenience
pub use torg_core::{BuildError, Builder, EvalError, Graph, Token};
pub use torg_mask::{
    ConstrainedDecoder, DecodeError, LogitMask, MaskGenerator, TokenMapping, TokenMappingBuilder,
};

/// Error types for TØR-G integration
#[derive(Debug, thiserror::Error)]
pub enum TorgError {
    #[error("Token mapping error: {0}")]
    TokenMapping(String),

    #[error("Build error: {0}")]
    Build(#[from] torg_core::BuildError),

    #[error("Decode error: {0}")]
    Decode(#[from] torg_mask::DecodeError),

    #[error("Evaluation error: {0}")]
    Eval(#[from] torg_core::EvalError),

    #[error("Invalid token ID: {0}")]
    InvalidToken(i32),
}
