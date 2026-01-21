use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentAuthError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Challenge request failed: {0}")]
    ChallengeRequest(String),

    #[error("Token request failed: {0}")]
    TokenRequest(String),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Agent not authorized: scan QR code with mobile app first")]
    NotAuthorized,

    #[error("Token expired")]
    TokenExpired,

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Crypto error: {0}")]
    Crypto(#[from] arkavo_crypto::CryptoError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
