use thiserror::Error;

/// JSON-RPC codes for the MCP PEP (draft-arkavo-authzen-cwt-00).
pub mod jsonrpc_codes {
    pub const DENIED: i32 = -32001;
    pub const MAPPING: i32 = -32602;
    pub const PDP: i32 = -32603;
}

#[derive(Error, Debug)]
pub enum AuthorizationError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Invalid configuration: {0}")]
    ConfigError(String),

    #[error("Authorization denied")]
    Denied,

    #[error("Mapping error: {0}")]
    Mapping(String),

    #[error("PDP unavailable: {0}")]
    PdpUnavailable(String),

    #[error("Invalid CWT: {0}")]
    InvalidToken(String),

    #[error("Timeout waiting for response")]
    Timeout,

    #[error("Service unavailable")]
    ServiceUnavailable,

    #[error("Invalid response from server: {0}")]
    InvalidResponse(String),
}

impl AuthorizationError {
    pub fn jsonrpc_code(&self) -> i32 {
        match self {
            Self::Denied => jsonrpc_codes::DENIED,
            Self::Mapping(_) | Self::InvalidToken(_) => jsonrpc_codes::MAPPING,
            Self::PdpUnavailable(_)
            | Self::ServiceUnavailable
            | Self::Timeout
            | Self::HttpError(_)
            | Self::InvalidResponse(_)
            | Self::SerializationError(_)
            | Self::ConfigError(_) => jsonrpc_codes::PDP,
        }
    }
}

pub type Result<T> = std::result::Result<T, AuthorizationError>;
