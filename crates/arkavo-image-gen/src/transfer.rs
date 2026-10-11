//! Assets over the data plane (ASSET-005, ASSET-006, ASSET-007).
//!
//! They are staged on this agent's node for the party that asked, fetched
//! from the member that staged them, and checked against their manifest at
//! both ends.

use std::path::{Path, PathBuf};
use std::time::Duration;

use arkavo_tdf_iroh::{IrohError, IrohTicket, IrohTransport};
use tokio::io::AsyncWriteExt;

use crate::delivery::{AssetRef, Delivery, PNG, safe_name, verify_fetched};
use crate::error::{ImageGenError, Result};
use crate::output::{AssetEntry, Generated};

/// How long a staged asset stays fetchable: the TDF offer TTL.
pub const RETENTION: Duration = Duration::from_hours(1);

/// Largest control-plane message that may carry references (PROTO-020).
pub const INLINE_LIMIT: usize = 64 * 1024;

/// Tries per asset before its producer is reported unreachable.
const ATTEMPTS: u32 = 3;

/// How patient a fetch is with the producer.
#[derive(Debug, Clone, Copy)]
pub struct FetchPolicy {
    /// Longest one attempt may take; a PNG of a few megabytes moves in
    /// seconds on a LAN, so this only bounds a producer that went silent.
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

/// Stage every asset of `generated` and say where each can be fetched.
pub async fn stage(transport: &IrohTransport, generated: &Generated) -> Result<Delivery> {
    let mut references = Vec::with_capacity(generated.manifest.assets.len());
    for asset in &generated.manifest.assets {
        let bytes = tokio::fs::read(generated.path_of(asset)).await?;
        // The file is staged as read now, so it is checked as read now.
        verify_fetched(asset, &bytes)?;
        let ticket = transport
            .stage_retained(&bytes, RETENTION)
            .await
            .map_err(staging_error)?;
        references.push(AssetRef {
            name: asset.name.clone(),
            ticket: ticket.to_string(),
            content_hash: ticket.hash().to_string(),
            media_type: PNG.to_string(),
        });
    }
    let delivery = Delivery {
        manifest: generated.manifest.clone(),
        references,
    };
    let size = serde_json::to_vec(&delivery).map_or(usize::MAX, |v| v.len());
    if size > INLINE_LIMIT {
        return Err(ImageGenError::Output(format!(
            "the references take {size} bytes, past the {INLINE_LIMIT}-byte inline bound"
        )));
    }
    Ok(delivery)
}

fn staging_error(error: IrohError) -> ImageGenError {
    match error {
        IrohError::StoreFull { .. } => ImageGenError::DataPlaneFull(error.to_string()),
        other => ImageGenError::DataPlane(other.to_string()),
    }
}

/// What fetching a delivery produced.
#[derive(Debug, Default)]
pub struct Fetched {
    /// Verified files, written under the fetch directory.
    pub saved: Vec<(AssetEntry, PathBuf)>,
    /// Assets that failed a fetch or a check, and why; none of their bytes
    /// were written.
    pub failed: Vec<(String, ImageGenError)>,
}

/// Fetch every referenced asset into `dir`, checking each against the
/// manifest before it is written. An existing file is never overwritten.
pub async fn fetch(
    transport: &IrohTransport,
    delivery: &Delivery,
    dir: &Path,
    policy: FetchPolicy,
) -> Fetched {
    let mut fetched = Fetched::default();
    for reference in &delivery.references {
        match fetch_one(transport, delivery, reference, dir, policy).await {
            Ok(saved) => fetched.saved.push(saved),
            Err(e) => fetched.failed.push((reference.name.clone(), e)),
        }
    }
    fetched
}

async fn fetch_one(
    transport: &IrohTransport,
    delivery: &Delivery,
    reference: &AssetRef,
    dir: &Path,
    policy: FetchPolicy,
) -> Result<(AssetEntry, PathBuf)> {
    let name = safe_name(&reference.name)?;
    let rejected = |reason: String| ImageGenError::Output(format!("{name}: {reason}"));
    let entry = delivery
        .entry(reference)
        .ok_or_else(|| rejected("not in the manifest".to_string()))?;
    if reference.media_type != PNG {
        return Err(rejected(format!("media type {}", reference.media_type)));
    }
    let ticket = IrohTicket::from_string(&reference.ticket).map_err(|e| rejected(e.to_string()))?;
    if ticket.hash().to_string() != reference.content_hash {
        return Err(rejected(
            "the ticket names a different blob than the reference".to_string(),
        ));
    }
    let bytes = fetch_bytes(transport, &ticket, name, policy).await?;
    verify_fetched(entry, &bytes)?;
    let path = dir.join(name);
    write_new(&path, &bytes).await?;
    Ok((entry.clone(), path))
}

async fn fetch_bytes(
    transport: &IrohTransport,
    ticket: &IrohTicket,
    name: &str,
    policy: FetchPolicy,
) -> Result<Vec<u8>> {
    let mut reason = String::new();
    let mut backoff = policy.backoff;
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(backoff).await;
            backoff *= 2;
        }
        match tokio::time::timeout(policy.attempt_timeout, transport.fetch_bytes(ticket)).await {
            Ok(Ok(bytes)) => return Ok(bytes),
            Ok(Err(e)) => reason = e.to_string(),
            Err(_) => {
                reason = format!("no answer within {}s", policy.attempt_timeout.as_secs_f64());
            }
        }
        tracing::debug!(asset = name, attempt, %reason, "asset fetch failed");
    }
    Err(ImageGenError::Unreachable {
        name: name.to_string(),
        reason: format!("{ATTEMPTS} attempts, last: {reason}"),
    })
}

async fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .map_err(|e| {
            ImageGenError::Output(format!("{}: {e}; nothing was overwritten", path.display()))
        })?;
    let written = async {
        file.write_all(bytes).await?;
        file.sync_all().await
    }
    .await;
    if let Err(e) = written {
        drop(file);
        let _ = tokio::fs::remove_file(path).await;
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
