use thiserror::Error;

#[derive(Error, Debug)]
pub enum BenchError {
    #[error("Task loading failed: {0}")]
    TaskLoadingFailed(String),

    #[error("Execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Evaluation failed: {0}")]
    EvaluationFailed(String),

    #[error("Workspace error: {0}")]
    Workspace(#[from] arkavo_workspace::WorkspaceError),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, BenchError>;
