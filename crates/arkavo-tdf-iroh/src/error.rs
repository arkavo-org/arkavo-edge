//! Error types for Iroh transport operations.

use thiserror::Error;

/// Errors specific to Iroh transport operations.
#[derive(Error, Debug)]
pub enum IrohError {
    /// Failed to initialize Iroh endpoint
    #[error("Endpoint initialization failed: {0}")]
    EndpointInit(String),

    /// Failed to start the blob protocol router
    #[error("Router failed: {0}")]
    Router(String),

    /// Failed to add blob to store
    #[error("Store add failed: {0}")]
    StoreAdd(String),

    /// Failed to export blob from store
    #[error("Store export failed: {0}")]
    StoreExport(String),

    /// Failed to download blob
    #[error("Download failed: {0}")]
    Download(String),

    /// Invalid ticket format
    #[error("Invalid ticket: {0}")]
    InvalidTicket(String),

    /// Node not running
    #[error("Node not running")]
    NodeNotRunning,

    /// Connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Shutdown error
    #[error("Shutdown error: {0}")]
    Shutdown(String),
}

impl From<IrohError> for arkavo_tdf::TransportError {
    fn from(err: IrohError) -> Self {
        match err {
            IrohError::StoreAdd(msg) => arkavo_tdf::TransportError::Stage(msg),
            IrohError::Download(msg) | IrohError::StoreExport(msg) => {
                arkavo_tdf::TransportError::Fetch(msg)
            }
            IrohError::InvalidTicket(msg) => arkavo_tdf::TransportError::InvalidTicket(msg),
            IrohError::NodeNotRunning | IrohError::EndpointInit(_) | IrohError::Router(_) => {
                arkavo_tdf::TransportError::NodeUnavailable(err.to_string())
            }
            IrohError::Connection(msg) => arkavo_tdf::TransportError::Connection(msg),
            IrohError::Io(e) => arkavo_tdf::TransportError::Io(e),
            IrohError::Shutdown(msg) => arkavo_tdf::TransportError::NodeUnavailable(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iroh_error_display() {
        let err = IrohError::EndpointInit("bind failed".to_string());
        assert!(err.to_string().contains("bind failed"));

        let err = IrohError::NodeNotRunning;
        assert_eq!(err.to_string(), "Node not running");
    }

    #[test]
    fn iroh_error_to_transport_error() {
        let iroh_err = IrohError::StoreAdd("disk full".to_string());
        let transport_err: arkavo_tdf::TransportError = iroh_err.into();
        assert!(matches!(transport_err, arkavo_tdf::TransportError::Stage(_)));

        let iroh_err = IrohError::InvalidTicket("bad format".to_string());
        let transport_err: arkavo_tdf::TransportError = iroh_err.into();
        assert!(matches!(
            transport_err,
            arkavo_tdf::TransportError::InvalidTicket(_)
        ));
    }
}
