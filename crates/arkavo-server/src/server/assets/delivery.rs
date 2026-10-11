//! Assets as they travel: their manifest, and one data-plane reference per
//! file (ASSET-005, ASSET-006, ASSET-007).
//!
//! Moving the bytes is the data plane's job (IROH-011); this module only
//! pairs each reference with its manifest entry and checks what arrives
//! against that entry.

use std::path::{Path, PathBuf};

use arkavo_image_gen::{
    ASSET_MANIFEST_SCHEMA, AssetEntry, Generated, ImageGenError, Manifest, PNG, safe_name,
    verify_fetched,
};
use arkavo_protocol::types::MessagePart;
use arkavo_tdf_iroh::{DataRef, FetchPolicy, INLINE_LIMIT, IrohError, IrohTransport, RETENTION};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

/// A manifest data part with schema [`ASSET_MANIFEST_SCHEMA`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    #[serde(flatten)]
    pub manifest: Manifest,
    pub references: Vec<DataRef>,
}

impl Delivery {
    /// The manifest entry a reference delivers.
    pub fn entry(&self, reference: &DataRef) -> Option<&AssetEntry> {
        self.manifest
            .assets
            .iter()
            .find(|a| a.name == reference.name)
    }

    /// As a data part for a task result.
    pub fn part(&self) -> Result<MessagePart, DeliveryError> {
        let content = serde_json::to_value(self)
            .map_err(|e| ImageGenError::Output(format!("manifest: {e}")))?;
        Ok(MessagePart::Data {
            schema: ASSET_MANIFEST_SCHEMA.to_string(),
            content,
        })
    }
}

/// Every delivery among a result's parts, in order. A part that does not
/// parse is left out and logged.
pub fn deliveries_in(parts: &[MessagePart]) -> Vec<Delivery> {
    parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Data { schema, content } if schema == ASSET_MANIFEST_SCHEMA => {
                match serde_json::from_value(content.clone()) {
                    Ok(delivery) => Some(delivery),
                    Err(e) => {
                        tracing::warn!(error = %e, "unreadable asset manifest");
                        None
                    }
                }
            }
            _ => None,
        })
        .collect()
}

/// Why assets were not delivered: their generation, or the data plane.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    #[error(transparent)]
    Asset(#[from] ImageGenError),
    #[error(transparent)]
    DataPlane(#[from] IrohError),
}

impl DeliveryError {
    /// The code requesters match on.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Asset(e) => e.code(),
            Self::DataPlane(IrohError::StoreFull { .. }) => "data_plane_full",
            Self::DataPlane(IrohError::Unreachable { .. }) => "asset_unreachable",
            Self::DataPlane(IrohError::RefMismatch { .. }) => "asset_rejected",
            Self::DataPlane(_) => "data_plane_unavailable",
        }
    }
}

/// Stage every asset of `generated` on this agent's data-plane node for an
/// hour, each checked again as it is read.
pub async fn stage(
    transport: &IrohTransport,
    generated: &Generated,
) -> Result<Delivery, DeliveryError> {
    let mut references = Vec::with_capacity(generated.manifest.assets.len());
    for asset in &generated.manifest.assets {
        let bytes = tokio::fs::read(generated.path_of(asset))
            .await
            .map_err(ImageGenError::from)?;
        verify_fetched(asset, &bytes)?;
        references.push(
            transport
                .stage_ref(&asset.name, PNG, &bytes, RETENTION)
                .await?,
        );
    }
    let delivery = Delivery {
        manifest: generated.manifest.clone(),
        references,
    };
    let size = serde_json::to_vec(&delivery).map_or(usize::MAX, |v| v.len());
    if size > INLINE_LIMIT {
        return Err(ImageGenError::Output(format!(
            "the references take {size} bytes, past the {INLINE_LIMIT}-byte inline bound"
        ))
        .into());
    }
    Ok(delivery)
}

/// What taking a delivery produced.
#[derive(Debug, Default)]
pub struct Fetched {
    /// Verified files, written under the delivery directory.
    pub saved: Vec<(AssetEntry, PathBuf)>,
    /// Assets that failed a fetch or a check; none of their bytes were
    /// written.
    pub failed: Vec<(String, DeliveryError)>,
}

/// Fetch every referenced asset into `dir`, checking each against its
/// manifest entry before it is written. An existing file is never
/// overwritten.
pub async fn take(
    transport: &IrohTransport,
    delivery: &Delivery,
    dir: &Path,
    policy: FetchPolicy,
) -> Fetched {
    let mut fetched = Fetched::default();
    for reference in &delivery.references {
        match take_one(transport, delivery, reference, dir, policy).await {
            Ok(saved) => fetched.saved.push(saved),
            Err(e) => fetched.failed.push((reference.name.clone(), e)),
        }
    }
    fetched
}

async fn take_one(
    transport: &IrohTransport,
    delivery: &Delivery,
    reference: &DataRef,
    dir: &Path,
    policy: FetchPolicy,
) -> Result<(AssetEntry, PathBuf), DeliveryError> {
    let name = safe_name(&reference.name)?;
    let rejected = |reason: String| ImageGenError::Output(format!("{name}: {reason}"));
    let entry = delivery
        .entry(reference)
        .ok_or_else(|| rejected("not in the manifest".to_string()))?;
    if reference.media_type != PNG || reference.size != entry.bytes {
        return Err(rejected(format!(
            "referenced as {} of {} bytes, listed as {PNG} of {}",
            reference.media_type, reference.size, entry.bytes
        ))
        .into());
    }
    let bytes = transport.fetch_ref(reference, policy).await?;
    verify_fetched(entry, &bytes)?;
    let path = dir.join(name);
    write_new(&path, &bytes).await?;
    Ok((entry.clone(), path))
}

async fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ImageGenError> {
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
// `#[tokio::test]` expands to `Runtime::block_on`, which the crate's lint set
// disallows in library code. Same waiver as the other async test modules here.
#[allow(clippy::disallowed_methods)]
mod tests;
