//! Error types for the Critic crate

use thiserror::Error;
use uuid::Uuid;

/// Result type alias for Critic operations
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during verification
#[derive(Debug, Error)]
pub enum Error {
    /// Schema validation failed
    #[error("Schema validation failed: {0}")]
    SchemaValidation(String),

    /// Policy check failed
    #[error("Policy violation: {policy} - {reason}")]
    PolicyViolation { policy: String, reason: String },

    /// Semantic check failed
    #[error("Semantic check failed: {0}")]
    SemanticFailure(String),

    /// Human approval required
    #[error("Human approval required for task {task_id}: {reason}")]
    HumanApprovalRequired { task_id: Uuid, reason: String },

    /// Human approval was denied
    #[error("Human approval denied for task {task_id}: {reason}")]
    HumanApprovalDenied { task_id: Uuid, reason: String },

    /// Check timed out
    #[error("Check '{check_id}' timed out after {timeout_ms}ms")]
    CheckTimeout { check_id: String, timeout_ms: u64 },

    /// Router/Judge error
    #[error("Router error: {0}")]
    Router(#[from] arkavo_router::Error),

    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}
