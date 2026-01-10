use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Invalid webhook signature")]
    InvalidSignature,

    #[error("Missing webhook secret")]
    MissingSecret,

    #[error("Invalid event type: {0}")]
    InvalidEventType(String),

    #[error("GitHub API error: {0}")]
    GitHubApi(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Task execution failed: {operation} - {details}")]
    TaskExecution { operation: String, details: String },

    #[error("Agent communication failed: {operation} with '{agent_id}' - {details}")]
    AgentCommunication {
        operation: String,
        agent_id: String,
        details: String,
    },

    #[error("Service discovery failed: {operation} - {details}")]
    Discovery { operation: String, details: String },

    #[error("Model error: {operation} - {details}")]
    Model { operation: String, details: String },

    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Protocol error: {0}")]
    Protocol(#[from] arkavo_protocol::error::A2aError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Memory store error: {0}")]
    Memory(#[from] arkavo_memory::error::MemoryError),

    #[error("Context error: {0}")]
    Context(#[from] arkavo_context::Error),

    #[error("Git error: {0}")]
    Git(#[from] arkavo_git::backend::GitError),

    #[error("GitHub error: {0}")]
    GitHub(#[from] arkavo_github::GitHubError),

    #[error("External error: {0}")]
    External(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
