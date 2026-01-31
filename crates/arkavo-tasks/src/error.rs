//! Error types for the task management system.

use thiserror::Error;

/// Errors that can occur during task execution, planning, or storage.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum TaskError {
    /// Task not found
    #[error("Task not found: {0}")]
    NotFound(String),

    /// Task store error
    #[error("Task store error: {0}")]
    Store(String),

    /// Task execution error
    #[error("Task execution error: {0}")]
    Execution(String),

    /// Invalid task state transition
    #[error("Invalid status transition from {from:?} to {to:?}")]
    InvalidTransition {
        /// Current status
        from: String,
        /// Attempted new status
        to: String,
    },

    /// Task timed out
    #[error("Task timed out after {0} seconds")]
    Timeout(u64),

    /// Task was cancelled
    #[error("Task was cancelled")]
    Cancelled,

    /// Task planning error
    #[error("Task planning error: {0}")]
    Planning(String),

    /// No agents available for task
    #[error("No agents available with required capability: {0}")]
    NoAgentsAvailable(String),

    /// Circular dependency detected
    #[error("Circular dependency detected in task plan")]
    CircularDependency,

    /// Invalid intent for task planning
    #[error("Invalid intent: {0}")]
    InvalidIntent(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type for task operations.
pub type Result<T> = std::result::Result<T, TaskError>;

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = TaskError::NotFound("task-123".to_string());
        assert_eq!(err.to_string(), "Task not found: task-123");

        let err = TaskError::Timeout(300);
        assert_eq!(err.to_string(), "Task timed out after 300 seconds");

        let err = TaskError::Cancelled;
        assert_eq!(err.to_string(), "Task was cancelled");

        let err = TaskError::NoAgentsAvailable("search".to_string());
        assert_eq!(
            err.to_string(),
            "No agents available with required capability: search"
        );
    }

    #[test]
    fn test_error_equality() {
        let err1 = TaskError::Cancelled;
        let err2 = TaskError::Cancelled;
        assert_eq!(err1, err2);

        let err3 = TaskError::Timeout(100);
        let err4 = TaskError::Timeout(100);
        assert_eq!(err3, err4);

        let err5 = TaskError::Timeout(200);
        assert_ne!(err3, err5);
    }
}
