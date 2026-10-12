//! References to staged data, as the control plane carries them in place of
//! the data (PROTO-020, IROH-011).
//!
//! A reference names what it delivers, the ticket to fetch it with, the
//! BLAKE3 hash the ticket names, its size and its media type. It means the
//! same whatever the bytes are: a generated image, a bundle, a document.

use std::time::Duration;

use iroh_base::EndpointAddr;
use iroh_blobs::HashAndFormat;
use iroh_blobs::api::blobs::BlobStatus;
use iroh_blobs::get::request::get_verified_size;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::error::IrohError;
use crate::ticket::IrohTicket;
use crate::transport::IrohTransport;

/// How long a reference handed out stays fetchable: the TDF offer TTL.
pub const RETENTION: Duration = Duration::from_hours(1);

/// Largest control-plane message that may carry references (PROTO-020).
pub const INLINE_LIMIT: usize = 64 * 1024;

/// Tries per reference before its provider is reported unreachable.
const ATTEMPTS: u32 = 3;

/// Where one piece of data can be fetched over the data plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataRef {
    /// What the data is to the party receiving it, such as a file name.
    pub name: String,
    /// Iroh blob ticket.
    pub ticket: String,
    /// BLAKE3 hash of the content, as the ticket names it.
    pub content_hash: String,
    pub size: u64,
    pub media_type: String,
}

/// How patient a fetch is with the provider.
#[derive(Debug, Clone, Copy)]
pub struct FetchPolicy {
    /// Longest one attempt may take; data of a few megabytes moves in
    /// seconds on a LAN, so this only bounds a provider that went silent.
    pub attempt_timeout: Duration,
    /// Wait before the second attempt, doubled before the third.
    pub backoff: Duration,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        Self {
            attempt_timeout: Duration::from_mins(2),
            backoff: Duration::from_secs(2),
        }
    }
}

impl IrohTransport {
    /// Stage `data` for another party for `retention` and describe it
    /// (IROH-009, IROH-010).
    pub async fn stage_ref(
        &self,
        name: &str,
        media_type: &str,
        data: &[u8],
        retention: Duration,
    ) -> Result<DataRef, IrohError> {
        let ticket = self.stage_retained(data, retention).await?;
        Ok(DataRef {
            name: name.to_string(),
            content_hash: ticket.hash().to_string(),
            ticket: ticket.to_string(),
            size: data.len() as u64,
            media_type: media_type.to_string(),
        })
    }

    /// Fetch exactly the data `reference` names (IROH-011).
    ///
    /// The ticket must name the reference's hash, and the provider's
    /// verified size must be the reference's before anything is downloaded,
    /// so a provider cannot hand over more than was agreed. A provider that
    /// does not answer is tried again with backoff.
    pub async fn fetch_ref(
        &self,
        reference: &DataRef,
        policy: FetchPolicy,
    ) -> Result<Vec<u8>, IrohError> {
        let ticket = IrohTicket::from_string(&reference.ticket)?;
        if ticket.hash().to_string() != reference.content_hash {
            return Err(IrohError::RefMismatch {
                name: reference.name.clone(),
                reason: "the ticket names a different blob".to_string(),
            });
        }
        if let Some(bytes) = self.held(&ticket, reference).await? {
            return Ok(bytes);
        }
        let mut reason = String::new();
        let mut backoff = policy.backoff;
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(backoff).await;
                backoff *= 2;
            }
            let fetched =
                tokio::time::timeout(policy.attempt_timeout, self.fetch_sized(&ticket, reference))
                    .await;
            match fetched {
                Ok(Ok(bytes)) => return Ok(bytes),
                Ok(Err(e @ IrohError::RefMismatch { .. })) => return Err(e),
                Ok(Err(e)) => reason = e.to_string(),
                Err(_) => {
                    reason = format!("no answer within {}s", policy.attempt_timeout.as_secs_f64());
                }
            }
            debug!(name = %reference.name, attempt, %reason, "data-plane fetch failed");
        }
        Err(IrohError::Unreachable {
            name: reference.name.clone(),
            reason: format!("{ATTEMPTS} attempts, last: {reason}"),
        })
    }

    /// The data when this node already holds the whole blob, staged here or
    /// fetched before. Iroh never connects a node to itself, and agents
    /// sharing one node in a process hand each other references to it.
    async fn held(
        &self,
        ticket: &IrohTicket,
        reference: &DataRef,
    ) -> Result<Option<Vec<u8>>, IrohError> {
        let store = self.node().store().await?;
        let failed = |e: &dyn std::fmt::Display| IrohError::StoreExport(e.to_string());
        // Tag first, so the collector cannot drop the blob between the
        // status check and the read.
        let batch = store.blobs().batch().await.map_err(|e| failed(&e))?;
        let _protected = batch
            .temp_tag(HashAndFormat {
                hash: ticket.hash(),
                format: ticket.inner().format(),
            })
            .await
            .map_err(|e| failed(&e))?;
        let status = store
            .blobs()
            .status(ticket.hash())
            .await
            .map_err(|e| failed(&e))?;
        match status {
            BlobStatus::Complete { size } if size == reference.size => {}
            BlobStatus::Complete { size } => {
                return Err(IrohError::RefMismatch {
                    name: reference.name.clone(),
                    reason: format!("{size} bytes held, {} referenced", reference.size),
                });
            }
            BlobStatus::Partial { .. } | BlobStatus::NotFound => return Ok(None),
        }
        let bytes = store
            .blobs()
            .get_bytes(ticket.hash())
            .await
            .map_err(|e| failed(&e))?;
        debug!(name = %reference.name, "data-plane reference read from this node");
        Ok(Some(bytes.to_vec()))
    }

    /// One attempt: the verified size first, then the bytes.
    async fn fetch_sized(
        &self,
        ticket: &IrohTicket,
        reference: &DataRef,
    ) -> Result<Vec<u8>, IrohError> {
        let size = self
            .verified_size(ticket.inner().addr().clone(), ticket)
            .await?;
        if size != reference.size {
            return Err(IrohError::RefMismatch {
                name: reference.name.clone(),
                reason: format!("{size} bytes offered, {} referenced", reference.size),
            });
        }
        let bytes = self.fetch_bytes(ticket).await?;
        if bytes.len() as u64 != reference.size {
            return Err(IrohError::RefMismatch {
                name: reference.name.clone(),
                reason: format!(
                    "{} bytes fetched, {} referenced",
                    bytes.len(),
                    reference.size
                ),
            });
        }
        Ok(bytes)
    }

    async fn verified_size(
        &self,
        addr: EndpointAddr,
        ticket: &IrohTicket,
    ) -> Result<u64, IrohError> {
        let endpoint = self.node().endpoint().await?;
        let connection = endpoint
            .connect(addr, iroh_blobs::protocol::ALPN)
            .await
            .map_err(|e| IrohError::Connection(e.to_string()))?;
        let (size, _) = get_verified_size(&connection, &ticket.hash())
            .await
            .map_err(|e| IrohError::Download(e.to_string()))?;
        Ok(size)
    }
}
