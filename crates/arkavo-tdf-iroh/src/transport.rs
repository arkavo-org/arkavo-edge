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
use crate::retention::sweep;
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

    /// Stage data for another party, fetchable for `retention` and then
    /// released to the garbage collector (IROH-010). Fails rather than evict
    /// a blob still in its window when the staged bytes would pass the
    /// node's bound (IROH-011).
    #[instrument(skip(self, data), fields(data_len = data.len()))]
    pub async fn stage_retained(
        &self,
        data: &[u8],
        retention: std::time::Duration,
    ) -> Result<IrohTicket, IrohError> {
        let store = self.node.store().await?;
        let endpoint = self.node.endpoint().await?;
        let retained = self.node.retention();
        sweep(&store, retained).await;
        let tag = retained.reserve(data.len() as u64, retention)?;
        let staged = match store.blobs().add_slice(data).with_named_tag(&tag).await {
            Ok(staged) => staged,
            Err(e) => {
                retained.release(&tag);
                return Err(IrohError::StoreAdd(e.to_string()));
            }
        };
        let ticket = BlobTicket::new(endpoint.addr(), staged.hash, staged.format);
        info!(%tag, ?retention, "retained blob staged");
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

        // A fetched blob has no tag of its own, so a temp tag keeps the
        // collector off it until it has been read.
        let batch = store
            .blobs()
            .batch()
            .await
            .map_err(|e| IrohError::Download(e.to_string()))?;
        let _protected = batch
            .temp_tag(iroh_blobs::HashAndFormat {
                hash: blob_ticket.hash(),
                format: blob_ticket.format(),
            })
            .await
            .map_err(|e| IrohError::Download(e.to_string()))?;

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
    use arkavo_test_macros::spec;
    use std::io::Cursor;

    #[spec("TDF-007", "IROH-002", "IROH-003")]
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

    #[spec("TDF-007")]
    #[tokio::test]
    async fn transport_invalid_ticket() {
        let node = IrohNode::memory().await.unwrap();
        let transport = IrohTransport::new(node);

        let err = transport.fetch("invalid-ticket").await.unwrap_err();
        match err {
            TransportError::InvalidTicket(msg) => {
                assert!(
                    msg.contains("Failed to parse ticket"),
                    "ticket parse context must be preserved, got {msg}"
                );
            }
            other => panic!("expected InvalidTicket, got {other}"),
        }

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

    #[spec("IROH-001")]
    #[tokio::test]
    async fn transport_new_node_started_and_ready() {
        let transport = IrohTransport::new(IrohNode::memory().await.unwrap());

        // Embedded node started: endpoint and blob store are initialized
        let node = transport.node();
        assert!(node.is_running().await);
        let _endpoint_id = node.endpoint_id().await.unwrap();
        let _store = node.store().await.unwrap();

        // Transport ready for operations
        assert!(transport.health_check().await.unwrap());

        transport.shutdown().await.unwrap();
    }

    #[spec("IROH-002")]
    #[tokio::test]
    async fn stage_empty_blob_produces_valid_ticket() {
        let transport = IrohTransport::new(IrohNode::memory().await.unwrap());

        let ticket = transport.stage(b"").await.unwrap();
        assert!(!ticket.is_empty());
        assert!(ticket.starts_with("blob"));

        // Zero-length blob is fetchable and content-addressed
        let fetched = transport.fetch(&ticket).await.unwrap();
        assert!(fetched.is_empty());

        // Same empty content yields the same content hash
        let ticket_again = transport.stage(b"").await.unwrap();
        let parsed = IrohTicket::from_string(&ticket).unwrap();
        let parsed_again = IrohTicket::from_string(&ticket_again).unwrap();
        assert_eq!(parsed.hash(), parsed_again.hash());

        transport.shutdown().await.unwrap();
    }

    #[spec("IROH-004")]
    #[tokio::test]
    async fn ticket_string_roundtrip_preserves_hash() {
        let transport = IrohTransport::new(IrohNode::memory().await.unwrap());

        let data = b"Ticket round-trip test";
        let ticket = transport.stage_bytes(data).await.unwrap();

        // Serialize to string and reconstruct
        let ticket_string = ticket.to_string();
        assert!(ticket_string.starts_with("blob"));
        let parsed = IrohTicket::from_string(&ticket_string).unwrap();

        // Round-trip preserves the content hash and node info
        assert_eq!(parsed.hash(), ticket.hash());
        assert_eq!(parsed.inner().addr().id, ticket.inner().addr().id);

        // Reconstructed ticket is usable for fetch
        let fetched = transport.fetch_bytes(&parsed).await.unwrap();
        assert_eq!(fetched, data);

        transport.shutdown().await.unwrap();
    }
}
