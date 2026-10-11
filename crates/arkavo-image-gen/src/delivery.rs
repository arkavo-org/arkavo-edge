//! What crosses the control plane for assets: requests and references,
//! never image bytes (ASSET-002, ASSET-005, ASSET-007, PROTO-020).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{ImageGenError, Result};
use crate::output::{AssetEntry, Manifest, png_header};
use crate::request::ImageRequest;

/// Most images one asset request may ask for.
pub const MAX_REQUEST_IMAGES: u32 = 16;

/// Media type of a delivered asset.
pub const PNG: &str = "image/png";

/// An asset request as a data part with schema
/// [`crate::ASSET_REQUEST_SCHEMA`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRequest {
    pub items: Vec<ImageRequest>,
    /// Seconds the member has to finish, from when it accepts.
    pub deadline_secs: u64,
    /// The agent asking, for the member's logs and the manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requester: Option<String>,
}

impl AssetRequest {
    /// Refuse an empty request or one past the image limit before any
    /// member is asked.
    pub fn check(&self) -> Result<()> {
        if self.items.is_empty() {
            return Err(ImageGenError::InvalidParams {
                field: "items",
                reason: "an asset request names at least one image".to_string(),
            });
        }
        let images: u32 = self.items.iter().map(|i| i.count.unwrap_or(1)).sum();
        if images > MAX_REQUEST_IMAGES {
            return Err(ImageGenError::InvalidParams {
                field: "items",
                reason: format!("{images} images requested; at most {MAX_REQUEST_IMAGES}"),
            });
        }
        Ok(())
    }
}

/// Where one asset can be fetched over the data plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetRef {
    /// The manifest entry this reference delivers.
    pub name: String,
    /// Iroh blob ticket.
    pub ticket: String,
    /// BLAKE3 content hash the ticket names.
    pub content_hash: String,
    pub media_type: String,
}

/// Assets as delivered: their manifest and where to fetch each one, as a
/// data part with schema [`crate::ASSET_MANIFEST_SCHEMA`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delivery {
    #[serde(flatten)]
    pub manifest: Manifest,
    pub references: Vec<AssetRef>,
}

impl Delivery {
    /// The manifest entry a reference delivers.
    pub fn entry(&self, reference: &AssetRef) -> Option<&AssetEntry> {
        self.manifest
            .assets
            .iter()
            .find(|a| a.name == reference.name)
    }
}

/// Check fetched bytes against the manifest entry they claim to be: size,
/// SHA-256, and a PNG of the stated size (ASSET-007).
pub fn verify_fetched(entry: &AssetEntry, bytes: &[u8]) -> Result<()> {
    use sha2::{Digest, Sha256};
    let reject = |reason: String| Err(ImageGenError::Output(format!("{}: {reason}", entry.name)));
    if bytes.len() as u64 != entry.bytes {
        return reject(format!(
            "{} bytes, manifest says {}",
            bytes.len(),
            entry.bytes
        ));
    }
    if hex::encode(Sha256::digest(bytes)) != entry.sha256 {
        return reject("SHA-256 differs from the manifest".to_string());
    }
    match png_header(bytes) {
        Some(h) if h.width == entry.width && h.height == entry.height => Ok(()),
        Some(h) => reject(format!(
            "{}x{}, manifest says {}x{}",
            h.width, h.height, entry.width, entry.height
        )),
        None => reject("not a PNG".to_string()),
    }
}

/// A delivered file name, refused when it could leave the directory it is
/// written to.
pub fn safe_name(name: &str) -> Result<&str> {
    let plain = !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !name.starts_with('.')
        && Path::new(name).extension().is_some_and(|e| e == "png");
    if plain {
        Ok(name)
    } else {
        Err(ImageGenError::Output(format!("unsafe asset name {name:?}")))
    }
}

#[cfg(test)]
mod tests;
