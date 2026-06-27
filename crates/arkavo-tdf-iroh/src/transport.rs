//! BlobTransport implementation using Iroh.
//!
//! Provides streaming blob staging and fetching via Iroh P2P network.

use arkavo_tdf::{BlobTransport, TransportError};
use async_trait::async_trait;
use iroh_blobs::ticket::BlobTicket;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, info, instrument};

use crate::error::IrohError;
use crate::node::IrohNode;
use crate::ticket::IrohTicket;

/// Iroh-based blob transport implementing the `BlobTransport` trait.
///
/// Handles raw byte transport over Iroh P2P network.
/// Does NOT perform encryption - that's composed at the application layer.
#[derive(Clone)]
pub struct IrohTransport {
    node: Arc<IrohNode>,
}

impl IrohTransport {
    /// Create a new transport wrapping an existing Iroh node.
    ///
    /// The node should already be started. Use `IrohNode::memory()` to create one:
    /// ```ignore
    /// let node = IrohNode::memory().await?;
    /// let transport = IrohTransport::new(node);
    /// ```
    pub fn new(node: Arc<IrohNode>) -> Self {
        Self { node }
    }

    /// Get the underlying Iroh node.
    #[must_use]
    pub fn node(&self) -> &Arc<IrohNode> {
        &self.node
    }

    /// Shutdown the transport gracefully.
    pub async fn shutdown(&self) -> Result<(), IrohError> {
        self.node.stop().await
    }

    /// Stage data and return an IrohTicket (typed wrapper).
    #[instrument(skip(self, data), fields(data_len = data.len()))]
    pub async fn stage_bytes(&self, data: &[u8]) -> Result<IrohTicket, IrohError> {
        let store = self.node.store().await?;
        let endpoint = self.node.endpoint().await?;

        debug!("Adding {} bytes to store", data.len());

        // Add data to store
        let tag = store
            .blobs()
            .add_slice(data)
            .await
            .map_err(|e| IrohError::StoreAdd(e.to_string()))?;

        debug!(?tag.hash, "Data stored");

        // Create ticket with full endpoint address (includes relay URL + direct addrs)
        let addr = endpoint.addr();
        let ticket = BlobTicket::new(addr, tag.hash, tag.format);

        info!(?ticket, "Ticket created");

        Ok(IrohTicket::new(ticket))
    }

    /// Fetch data using an IrohTicket (typed wrapper).
    #[instrument(skip(self))]
    pub async fn fetch_bytes(&self, ticket: &IrohTicket) -> Result<Vec<u8>, IrohError> {
        let store = self.node.store().await?;
        let endpoint = self.node.endpoint().await?;
        let blob_ticket = ticket.inner();

        debug!(?blob_ticket, "Fetching blob");

        // Establish a direct connection to the provider using the full address
        // embedded in the ticket (direct IPs + relay URL). The downloader below
        // resolves providers by bare EndpointId, which on its own can only be
        // located via the endpoint's external AddressLookup (n0.computer DNS) —
        // that path needs outbound network and fails on restricted CI runners.
        // Pre-connecting here seeds the connection pool with a live connection
        // that get_or_connect then reuses, so the fetch succeeds via loopback
        // without any external resolution. Best-effort: if the direct connect
        // itself fails, fall through and let the downloader try relay/lookup.
        if let Err(e) = endpoint
            .connect(blob_ticket.addr().clone(), iroh_blobs::protocol::ALPN)
            .await
        {
            debug!(error = %e, "Direct connect to provider failed; falling back to downloader resolution");
        }

        // Create downloader and download blob
        let downloader = store.downloader(&endpoint);

        // Download blob from remote peer
        downloader
            .download(blob_ticket.hash(), Some(blob_ticket.addr().id))
            .await
            .map_err(|e| IrohError::Download(e.to_string()))?;

        debug!("Download complete, reading bytes");

        // Read blob bytes using the reader API
        let mut reader = store.blobs().reader(blob_ticket.hash());
        let mut output = Vec::new();
        reader
            .read_to_end(&mut output)
            .await
            .map_err(|e| IrohError::StoreExport(e.to_string()))?;

        info!(bytes = output.len(), "Blob fetched");

        Ok(output)
    }
}

#[async_trait]
impl BlobTransport for IrohTransport {
    #[instrument(skip(self, reader))]
    async fn stage_stream<R>(&self, mut reader: R) -> Result<String, TransportError>
    where
        R: AsyncRead + Send + Unpin,
    {
        // Read all data from stream
        // Note: For very large files, we'd want to use Iroh's streaming add API
        let mut buffer = Vec::new();
        reader.read_to_end(&mut buffer).await?;

        let ticket = self.stage_bytes(&buffer).await?;
        Ok(ticket.to_string())
    }

    #[instrument(skip(self, writer))]
    async fn fetch_stream<W>(&self, ticket: &str, mut writer: W) -> Result<u64, TransportError>
    where
        W: AsyncWrite + Send + Unpin,
    {
        let iroh_ticket = IrohTicket::from_string(ticket)?;
        let data = self.fetch_bytes(&iroh_ticket).await?;
        let len = data.len() as u64;

        writer.write_all(&data).await?;

        Ok(len)
    }

    async fn stage(&self, data: &[u8]) -> Result<String, TransportError> {
        let ticket = self.stage_bytes(data).await?;
        Ok(ticket.to_string())
    }

    async fn fetch(&self, ticket: &str) -> Result<Vec<u8>, TransportError> {
        let iroh_ticket = IrohTicket::from_string(ticket)?;
        let data = self.fetch_bytes(&iroh_ticket).await?;
        Ok(data)
    }

    async fn health_check(&self) -> Result<bool, TransportError> {
        Ok(self.node.is_running().await)
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[tokio::test]
    async fn transport_stage_fetch_roundtrip() {
        let node = IrohNode::memory().await.unwrap();
        let transport = IrohTransport::new(node.clone());

        let data = b"Hello, Iroh!";
        let ticket = transport.stage(data).await.unwrap();

        assert!(!ticket.is_empty());
        assert!(ticket.starts_with("blob")); // Iroh tickets start with "blob"

        // Fetch from same node (self-fetch test)
        let fetched = transport.fetch(&ticket).await.unwrap();
        assert_eq!(fetched, data);

        transport.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn transport_streaming() {
        let node = IrohNode::memory().await.unwrap();
        let transport = IrohTransport::new(node);

        let data = b"Streaming data test";
        let reader = Cursor::new(data.to_vec());

        let ticket = transport.stage_stream(reader).await.unwrap();

        let mut output = Vec::new();
        let bytes = transport.fetch_stream(&ticket, &mut output).await.unwrap();

        assert_eq!(bytes, data.len() as u64);
        assert_eq!(output, data);

        transport.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn transport_health_check() {
        let node = IrohNode::memory().await.unwrap();
        let transport = IrohTransport::new(node);

        assert!(transport.health_check().await.unwrap());

        transport.shutdown().await.unwrap();

        // After shutdown, health check should return false
        assert!(!transport.health_check().await.unwrap());
    }

    #[tokio::test]
    async fn transport_invalid_ticket() {
        let node = IrohNode::memory().await.unwrap();
        let transport = IrohTransport::new(node);

        let result = transport.fetch("invalid-ticket").await;
        assert!(result.is_err());

        transport.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn transport_shared_node() {
        let node = IrohNode::memory().await.unwrap();
        let transport = IrohTransport::new(node.clone());

        let data = b"Shared node test";
        let ticket = transport.stage(data).await.unwrap();
        let fetched = transport.fetch(&ticket).await.unwrap();

        assert_eq!(fetched, data);

        // Shutdown via node directly
        node.stop().await.unwrap();
    }
}
