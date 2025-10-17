use thiserror::Error;

#[derive(Error, Debug)]
pub enum CefError {
    #[error("CEF process failed to start: {0}")]
    ProcessStartFailed(String),

    #[error("CEF process crashed: {0}")]
    ProcessCrashed(String),

    #[error("UDS connection failed: {0}")]
    UdsConnectionFailed(String),

    #[error("UDS transport error: {0}")]
    UdsTransportError(String),

    #[error("DOM command failed: {0}")]
    DomCommandFailed(String),

    #[error("Invalid DOM selector: {0}")]
    InvalidSelector(String),

    #[error("Blink DOM exception: {0}")]
    BlinkException(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("Timeout waiting for response")]
    Timeout,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CefError>;
