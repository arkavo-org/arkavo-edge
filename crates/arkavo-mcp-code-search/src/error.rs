use thiserror::Error;

#[derive(Error, Debug)]
pub enum CodeSearchError {
    #[error("Ripgrep command failed: {0}")]
    RipgrepError(String),

    #[error("Invalid pattern: {0}")]
    InvalidPattern(String),

    #[error("Path not found: {0}")]
    PathNotFound(String),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Path outside the workspace: {0}")]
    OutsideWorkspace(String),

    #[error("Tool error: {0}")]
    ToolError(String),
}

pub type Result<T> = std::result::Result<T, CodeSearchError>;

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;
    use arkavo_mcp::Tool;
    use arkavo_test_macros::spec;

    #[spec("CS-005")]
    #[test]
    fn io_error_converts_preserving_context_and_message() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "reading /tmp/x.rs failed");
        let err: CodeSearchError = io_err.into();
        match &err {
            CodeSearchError::IoError(inner) => {
                assert_eq!(inner.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected IoError, got {other:?}"),
        }
        let message = err.to_string();
        assert!(message.starts_with("I/O error:"), "{message}");
        assert!(message.contains("reading /tmp/x.rs failed"), "{message}");
    }

    #[spec("CS-005")]
    #[tokio::test]
    async fn missing_pattern_returns_invalid_pattern_with_explanation() {
        let tool = crate::CodeGrepTool::with_root(".");
        let err = tool.execute(serde_json::json!({})).await.unwrap_err();
        let code_search_err = err
            .downcast_ref::<CodeSearchError>()
            .expect("error must be a CodeSearchError");
        match code_search_err {
            CodeSearchError::InvalidPattern(message) => {
                assert!(message.contains("Missing pattern"), "{message}");
            }
            other => panic!("expected InvalidPattern, got {other:?}"),
        }
    }
}
