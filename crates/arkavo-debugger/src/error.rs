use thiserror::Error;

#[derive(Error, Debug)]
pub enum DebuggerError {
    #[error("Event not found: {0}")]
    EventNotFound(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Storage error: {0}")]
    Storage(#[from] arkavo_memory::error::MemoryError),

    #[error("Event error: {0}")]
    Event(#[from] arkavo_events::EventError),

    #[error("Replay error: {0}")]
    Replay(String),

    #[error("Analysis error: {0}")]
    Analysis(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Parse error: {0}")]
    Parse(String),
}

pub type Result<T> = std::result::Result<T, DebuggerError>;

/// Alias for compatibility with SessionManager
pub type DebugError = DebuggerError;

impl From<uuid::Error> for DebuggerError {
    fn from(err: uuid::Error) -> Self {
        DebuggerError::Parse(err.to_string())
    }
}

impl From<chrono::ParseError> for DebuggerError {
    fn from(err: chrono::ParseError) -> Self {
        DebuggerError::Parse(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("DBG-006")]
    #[test]
    fn test_error_context_captured() {
        let err = DebuggerError::SessionNotFound("session-42".to_string());
        let msg = err.to_string();
        assert!(msg.contains("session-42"));

        let err = DebuggerError::Replay("Events from multiple sessions".to_string());
        let msg = err.to_string();
        assert!(msg.contains("Replay error"));
        assert!(msg.contains("Events from multiple sessions"));

        let err = DebuggerError::EventNotFound("event-7".to_string());
        assert!(err.to_string().contains("event-7"));
    }

    #[spec("DBG-006")]
    #[test]
    fn test_error_conversion_propagates_context() {
        // A failed parse deep in the stack converts into a DebuggerError
        // that still carries the original failure detail
        let uuid_err = "not-a-uuid".parse::<uuid::Uuid>().unwrap_err();
        let err = DebuggerError::from(uuid_err);
        assert!(matches!(err, DebuggerError::Parse(_)));
        assert!(!err.to_string().is_empty());

        let chrono_err = "not-a-timestamp"
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap_err();
        let err = DebuggerError::from(chrono_err);
        assert!(matches!(err, DebuggerError::Parse(_)));

        // The Result alias propagates the error unchanged
        fn fallible() -> Result<()> {
            Err(DebuggerError::Analysis("no events".to_string()))
        }
        let err = fallible().unwrap_err();
        assert!(matches!(err, DebuggerError::Analysis(_)));
        assert!(err.to_string().contains("no events"));
    }
}
