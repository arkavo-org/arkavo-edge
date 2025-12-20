//! Error types for the auto-learning loop

use thiserror::Error;
use uuid::Uuid;

/// Errors that can occur during auto-learning
#[derive(Debug, Error)]
pub enum AutoLearnError {
    /// Synthesis failed
    #[error("Synthesis failed: {0}")]
    Synthesis(String),

    /// Verification failed
    #[error("Verification failed: {0}")]
    Verification(String),

    /// Network error
    #[error("Network error: {0}")]
    Network(String),

    /// Gossip protocol error
    #[error("Gossip error: {0}")]
    Gossip(#[from] arkavo_gossip::GossipError),

    /// Ensemble error
    #[error("Ensemble error: {0}")]
    Ensemble(#[from] arkavo_ensemble::EnsembleError),

    /// SAT verification error
    #[error("SAT error: {0}")]
    Sat(String),

    /// Invalid patchlet
    #[error("Invalid patchlet: {0}")]
    InvalidPatchlet(String),

    /// Patch not found
    #[error("Patch not found: {0}")]
    PatchNotFound(Uuid),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Timeout error
    #[error("Operation timed out")]
    Timeout,

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),

    /// Model not loaded (LLM synthesis requires loading a model)
    #[error("Model not loaded: call with_model() to load an LLM")]
    ModelNotLoaded,

    /// TØR-G constrained decoding error
    #[cfg(feature = "llm")]
    #[error("TØR-G error: {0}")]
    Torg(#[from] arkavo_torg::TorgError),

    /// LLM inference error
    #[cfg(feature = "llm")]
    #[error("LLM error: {0}")]
    Llm(String),
}

/// Result type for auto-learning operations
pub type AutoLearnResult<T> = Result<T, AutoLearnError>;

impl From<serde_json::Error> for AutoLearnError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e.to_string())
    }
}

impl From<tokio::time::error::Elapsed> for AutoLearnError {
    fn from(_: tokio::time::error::Elapsed) -> Self {
        Self::Timeout
    }
}
