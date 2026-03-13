use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("parse error: {0}")]
    Parse(String),

    #[error("render error: {0}")]
    Render(String),

    #[error("compile error: {stderr}")]
    Compile { stderr: String },

    #[error("test error: {stderr}")]
    Test { stderr: String },

    #[error("target not found: {0}")]
    TargetNotFound(String),

    #[error("ambiguous target: {0} matches found for {1}")]
    AmbiguousTarget(usize, String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("capability denied: {0}")]
    CapabilityDenied(String),

    #[error("clock violation: {0}")]
    ClockViolation(String),

    #[error("anchor failed: {0}")]
    AnchorFailed(String),

    #[error("version not found: {0}")]
    VersionNotFound(String),
}
