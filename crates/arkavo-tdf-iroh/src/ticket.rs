//! Ticket handling for Iroh blob transfers.
//!
//! Wraps `iroh_blobs::ticket::BlobTicket` with serialization helpers.

use iroh_blobs::ticket::BlobTicket;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

use crate::error::IrohError;

/// Wrapper around Iroh's BlobTicket for transport over control plane.
///
/// Tickets contain:
/// - Node ID (endpoint address)
/// - Blob hash (BLAKE3 content address)
/// - Blob format (raw or hash sequence)
#[derive(Debug, Clone)]
pub struct IrohTicket {
    inner: BlobTicket,
}

impl IrohTicket {
    /// Create a new ticket from an Iroh BlobTicket.
    #[must_use]
    pub fn new(ticket: BlobTicket) -> Self {
        Self { inner: ticket }
    }

    /// Get the underlying BlobTicket.
    #[must_use]
    pub fn inner(&self) -> &BlobTicket {
        &self.inner
    }

    /// Convert to the underlying BlobTicket.
    #[must_use]
    pub fn into_inner(self) -> BlobTicket {
        self.inner
    }

    /// Get the BLAKE3 content hash of the blob.
    #[must_use]
    pub fn hash(&self) -> iroh_blobs::Hash {
        self.inner.hash()
    }

    /// Parse from a string received over control plane.
    pub fn from_string(s: &str) -> Result<Self, IrohError> {
        let ticket = BlobTicket::from_str(s)
            .map_err(|e| IrohError::InvalidTicket(format!("Failed to parse ticket: {e}")))?;
        Ok(Self { inner: ticket })
    }
}

impl std::fmt::Display for IrohTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl FromStr for IrohTicket {
    type Err = IrohError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_string(s)
    }
}

impl From<BlobTicket> for IrohTicket {
    fn from(ticket: BlobTicket) -> Self {
        Self::new(ticket)
    }
}

impl From<IrohTicket> for BlobTicket {
    fn from(ticket: IrohTicket) -> Self {
        ticket.inner
    }
}

/// JSON-serializable ticket for embedding in messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableTicket {
    /// The ticket string
    pub ticket: String,
}

impl SerializableTicket {
    /// Create from an IrohTicket.
    #[must_use]
    pub fn from_ticket(ticket: &IrohTicket) -> Self {
        Self {
            ticket: ticket.to_string(),
        }
    }

    /// Parse into an IrohTicket.
    pub fn to_ticket(&self) -> Result<IrohTicket, IrohError> {
        IrohTicket::from_string(&self.ticket)
    }
}

impl From<&IrohTicket> for SerializableTicket {
    fn from(ticket: &IrohTicket) -> Self {
        Self::from_ticket(ticket)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializable_ticket_roundtrip() {
        let json = r#"{"ticket":"test-ticket-string"}"#;
        let parsed: SerializableTicket = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.ticket, "test-ticket-string");

        let serialized = serde_json::to_string(&parsed).unwrap();
        assert!(serialized.contains("test-ticket-string"));
    }

    #[test]
    fn invalid_ticket_error() {
        let result = IrohTicket::from_string("not-a-valid-ticket");
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(matches!(err, IrohError::InvalidTicket(_)));
    }
}
