use thiserror::Error;

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("GitHub API error: {0}")]
    GitHubApi(String),

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
}

pub type Result<T> = std::result::Result<T, DiscoveryError>;
