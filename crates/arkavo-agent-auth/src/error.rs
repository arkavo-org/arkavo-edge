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

    #[error("Identity will not issue this agent a token: {0}")]
    Refused(String),

    #[error("Identity refused to authorize this agent: {0}")]
    AuthorizeRefused(String),

    #[error("Identity did not accept the owner's sign-in: {0}")]
    OwnerSignInRejected(String),

    #[error("Authorize request failed: {0}")]
    AuthorizeRequest(String),

    #[error("Identity refused to revoke this agent: {0}")]
    RevokeRefused(String),

    #[error("Revoke request failed: {0}")]
    RevokeRequest(String),

    #[error("Token expired")]
    TokenExpired,

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Crypto error: {0}")]
    Crypto(#[from] arkavo_crypto::CryptoError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
