use thiserror::Error;

#[derive(Error, Debug)]
pub enum BrowserError {
    #[error("Playwright error: {0}")]
    Playwright(String),

    #[error("Page navigation failed: {0}")]
    Navigation(String),

    #[error("Element not found: {0}")]
    ElementNotFound(String),

    #[error("Screenshot failed: {0}")]
    Screenshot(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Invalid parameters: {0}")]
    InvalidParams(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, BrowserError>;
