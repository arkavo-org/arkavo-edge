//! Error types for TDF operations.

use thiserror::Error;

/// Errors that can occur during TDF encryption/decryption operations.
#[derive(Error, Debug)]
pub enum TdfError {
    /// Key Access Service (KAS) communication error
    #[error("KAS error: {0}")]
    Kas(String),

    /// Policy validation or parsing error
    #[error("Policy error: {0}")]
    Policy(String),

    /// Encryption operation failed
    #[error("Encryption error: {0}")]
    Encryption(String),

    /// Decryption operation failed
    #[error("Decryption error: {0}")]
    Decryption(String),

    /// Manifest parsing or validation error
    #[error("Manifest error: {0}")]
    Manifest(String),

    /// I/O error during streaming operations
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization/deserialization error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Base64 encoding/decoding error
    #[error("Base64 error: {0}")]
    Base64(String),

    /// Invalid configuration
    #[error("Configuration error: {0}")]
    Config(String),

    /// Authentication/authorization failure
    #[error("Auth error: {0}")]
    Auth(String),

    /// Operation timeout
    #[error("Timeout: {0}")]
    Timeout(String),
}

/// Errors that can occur during blob transport operations.
#[derive(Error, Debug)]
pub enum TransportError {
    /// Failed to stage blob
    #[error("Stage error: {0}")]
    Stage(String),

    /// Failed to fetch blob
    #[error("Fetch error: {0}")]
    Fetch(String),

    /// Invalid ticket format
    #[error("Invalid ticket: {0}")]
    InvalidTicket(String),

    /// Transport node not available
    #[error("Node unavailable: {0}")]
    NodeUnavailable(String),

    /// I/O error during streaming
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// Operation timeout
    #[error("Timeout: {0}")]
    Timeout(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tdf_error_display() {
        let err = TdfError::Kas("connection refused".to_string());
        assert_eq!(err.to_string(), "KAS error: connection refused");

        let err = TdfError::Policy("invalid attribute".to_string());
        assert_eq!(err.to_string(), "Policy error: invalid attribute");
    }

    #[test]
    fn transport_error_display() {
        let err = TransportError::Stage("disk full".to_string());
        assert_eq!(err.to_string(), "Stage error: disk full");

        let err = TransportError::InvalidTicket("malformed base64".to_string());
        assert_eq!(err.to_string(), "Invalid ticket: malformed base64");
    }

    #[test]
    fn tdf_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let tdf_err: TdfError = io_err.into();
        assert!(matches!(tdf_err, TdfError::Io(_)));
    }

    #[test]
    fn transport_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe broken");
        let transport_err: TransportError = io_err.into();
        assert!(matches!(transport_err, TransportError::Io(_)));
    }
}
