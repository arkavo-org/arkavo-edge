//! Error types for HRM orchestration

use thiserror::Error;
use uuid::Uuid;

/// Result type for HRM operations
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during HRM orchestration
#[derive(Debug, Error)]
pub enum Error {
    /// Task not found in store
    #[error("Task not found: {0}")]
    TaskNotFound(Uuid),

    /// SubTask not found
    #[error("SubTask not found: {0}")]
    SubTaskNotFound(Uuid),

    /// Task budget exceeded
    #[error("Task budget exceeded: {reason}")]
    BudgetExceeded { reason: String },

    /// Burst contract expired
    #[error("Burst contract expired: {contract_id}")]
    ContractExpired { contract_id: Uuid },

    /// Strategic thrashing detected - repeated failures on semantically similar subtasks
    #[error("Strategic thrashing detected: subtask '{description}' failed {attempts} times")]
    StrategicThrashing {
        task_id: Uuid,
        description: String,
        attempts: u32,
    },

    /// Maximum subtask limit reached
    #[error("Maximum subtask limit reached: {current}/{max}")]
    MaxSubtasksExceeded { current: u32, max: u32 },

    /// Maximum recursion depth reached
    #[error("Maximum recursion depth reached: {depth}")]
    MaxRecursionDepth { depth: u32 },

    /// Task is in an invalid state for the requested operation
    #[error("Invalid task state: expected {expected}, got {actual}")]
    InvalidTaskState { expected: String, actual: String },

    /// Persistence error
    #[error("Store error: {0}")]
    StoreError(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Context strategy failed
    #[error("Context strategy failed: {0}")]
    ContextStrategyError(String),

    /// Context ledger error
    #[error("Context ledger error: {0}")]
    Context(String),

    /// Task was cancelled
    #[error("Task cancelled: {0}")]
    TaskCancelled(Uuid),

    /// Task is suspended awaiting approval
    #[error("Task suspended awaiting approval: {0}")]
    AwaitingApproval(Uuid),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

impl Error {
    /// Create a store error from any error type
    pub fn store<E: std::fmt::Display>(err: E) -> Self {
        Self::StoreError(err.to_string())
    }

    /// Create an internal error
    pub fn internal<S: Into<String>>(msg: S) -> Self {
        Self::Internal(msg.into())
    }

    /// Check if this error indicates the task should be retried
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::ContractExpired { .. } | Self::StoreError(_) | Self::Internal(_)
        )
    }

    /// Check if this error is a terminal failure
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::StrategicThrashing { .. }
                | Self::MaxSubtasksExceeded { .. }
                | Self::MaxRecursionDepth { .. }
                | Self::TaskCancelled(_)
        )
    }
}
