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

    #[error("Discovery error: {0}")]
    Discovery(#[from] arkavo_org_discovery::DiscoveryError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
