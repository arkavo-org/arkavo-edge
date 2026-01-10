use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitHubError {
    #[error("GitHub API error: {0}")]
    GitHubApi(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Octocrab error: {0}")]
    Octocrab(#[from] Box<octocrab::Error>),

    #[error("Invalid regex pattern: {0}")]
    InvalidRegex(#[from] regex::Error),

    #[error("Organization not found: {0}")]
    OrgNotFound(String),

    #[error("Rate limit exceeded, retry after {0} seconds")]
    RateLimitExceeded(u64),

    #[error("Cache error: {0}")]
    CacheError(String),

    #[error("No repositories found for organization: {0}")]
    NoRepositories(String),

    #[error("Memory store error: {0}")]
    Memory(#[from] arkavo_memory::error::MemoryError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Anyhow error: {0}")]
    Anyhow(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, GitHubError>;
