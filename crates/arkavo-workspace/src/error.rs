use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkspaceError {
    #[error("Container runtime error: {0}")]
    ContainerRuntime(String),

    #[error("Workspace creation failed: {0}")]
    CreationFailed(String),

    #[error("Resource limit exceeded: {0}")]
    ResourceLimit(String),

    #[error("Workspace not found: {0}")]
    NotFound(String),

    #[error("Cleanup failed: {0}")]
    CleanupFailed(String),

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, WorkspaceError>;
