//! Error types for the ensemble crate

use thiserror::Error;
use uuid::Uuid;

/// Result type for ensemble operations
pub type EnsembleResult<T> = Result<T, EnsembleError>;

/// Errors that can occur during ensemble operations
#[derive(Debug, Error)]
pub enum EnsembleError {
    /// Candidate not found
    #[error("Candidate not found: {0}")]
    CandidateNotFound(Uuid),

    /// Maximum candidates reached
    #[error("Maximum candidates reached: {max}")]
    MaxCandidatesReached { max: usize },

    /// Invariant violation during evaluation
    #[error("Invariant violation for candidate {candidate_id}: {description}")]
    InvariantViolation {
        candidate_id: Uuid,
        description: String,
    },

    /// Evaluation error
    #[error("Evaluation error: {0}")]
    Evaluation(String),

    /// Statistical error (insufficient data, etc.)
    #[error("Statistical error: {0}")]
    Statistical(String),

    /// Promotion requirements not met
    #[error("Promotion requirements not met: {0}")]
    PromotionFailed(String),

    /// SAT verification failed
    #[error("SAT verification failed: {0}")]
    SatVerification(String),

    /// Storage error
    #[error("Storage error: {0}")]
    Storage(String),

    /// Synthesis error
    #[error("Synthesis error: {0}")]
    Synthesis(String),

    /// SBE layer error
    #[error("SBE error: {0}")]
    Sbe(#[from] arkavo_sbe::SbeError),

    /// SAT solver error
    #[error("SAT error: {0}")]
    Sat(#[from] arkavo_sat::SatError),
}
