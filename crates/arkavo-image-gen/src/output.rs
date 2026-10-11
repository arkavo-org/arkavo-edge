//! Checking what sd-cli wrote, and describing it (IMG-009, IMG-010,
//! IMG-013).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::{RUNTIME, WeightSet};
use crate::error::{ImageGenError, Result};
use crate::fetch::sha256_file;
use crate::request::Job;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// One generated image, as requesters see it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetEntry {
    /// File name, such as `coin_000.png`.
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub tileable: bool,
    pub seed: u32,
}

/// The catalog entry that produced a set of assets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUsed {
    pub id: String,
    pub repo: String,
    pub revision: String,
    pub diffusion: String,
    pub text_encoder: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// What generated a set of assets and under which terms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub model: ModelUsed,
    pub license: String,
    pub license_url: String,
    pub runtime: String,
    /// The did:key of the agent that generated the assets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    /// Set when transparency was asked for and an image came back without it.
    #[serde(default)]
    pub transparency_missing: bool,
    pub assets: Vec<AssetEntry>,
}

impl Manifest {
    /// One line naming the model and its license, shown with every delivery.
    pub fn license_line(&self) -> String {
        format!(
            "Generated locally with {} ({}); license: {} <{}>",
            self.model.id, self.runtime, self.license, self.license_url
        )
    }
}

/// Assets on disk with their manifest; the caller owns `dir` and deletes it
/// once the files are delivered.
#[derive(Debug, Clone)]
pub struct Generated {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

impl Generated {
    pub fn path_of(&self, asset: &AssetEntry) -> PathBuf {
        self.dir.join(&asset.name)
    }
}

/// Verify sd-cli's output in `dir` for `job` and describe it.
pub async fn collect(dir: &Path, job: &Job, set: &WeightSet, seeds: &[u32]) -> Result<Manifest> {
    let mut assets = Vec::with_capacity(job.count as usize);
    for index in 0..job.count {
        let name = format!("{}_{index:03}.png", job.name);
        let path = dir.join(&name);
        let header = read_header(&path).await?;
        if header.width != job.width || header.height != job.height {
            return Err(ImageGenError::Output(format!(
                "{name} is {}x{}, {}x{} was requested",
                header.width, header.height, job.width, job.height
            )));
        }
        assets.push(AssetEntry {
            bytes: tokio::fs::metadata(&path).await?.len(),
            sha256: sha256_file(&path).await?,
            name,
            width: header.width,
            height: header.height,
            has_alpha: header.has_alpha,
            tileable: job.tileable,
            seed: seeds
                .get(index as usize)
                .copied()
                .unwrap_or_else(|| job.seed.wrapping_add(index)),
        });
    }
    let extra = count_pngs(dir).await?;
    if extra != assets.len() {
        return Err(ImageGenError::Output(format!(
            "expected {} images, found {extra}",
            assets.len()
        )));
    }
    let file_name = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    Ok(Manifest {
        model: ModelUsed {
            id: job.entry.id.to_string(),
            repo: job.entry.model_repo.to_string(),
            revision: job.entry.model_revision.to_string(),
            diffusion: file_name(set.diffusion.path),
            text_encoder: file_name(set.text_encoder.path),
            alias: job.alias.map(str::to_string),
        },
        license: job.entry.license.to_string(),
        license_url: job.entry.license_url.to_string(),
        runtime: format!("stable-diffusion.cpp {}", RUNTIME.tag),
        producer: None,
        prompt: job.prompt.clone(),
        guidance: job.guided().then_some(job.guidance),
        negative_prompt: job.negative_prompt.clone(),
        transparency_missing: job.transparent && assets.iter().any(|a| !a.has_alpha),
        assets,
    })
}

async fn count_pngs(dir: &Path) -> Result<usize> {
    let mut entries = tokio::fs::read_dir(dir).await?;
    let mut count = 0;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name().to_string_lossy().ends_with(".png") {
            count += 1;
        }
    }
    Ok(count)
}

/// The facts about a PNG the manifest needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngHeader {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
}

async fn read_header(path: &Path) -> Result<PngHeader> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| ImageGenError::Output(format!("{} was not written: {e}", path.display())))?;
    png_header(&bytes)
        .ok_or_else(|| ImageGenError::Output(format!("{} is not a PNG", path.display())))
}

/// Parse the signature and IHDR, and look for a tRNS chunk before the image
/// data; `None` for anything that is not a well-formed PNG start.
pub fn png_header(bytes: &[u8]) -> Option<PngHeader> {
    if bytes.get(..8)? != PNG_SIGNATURE || bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let be = |at: usize| Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let (width, height) = (be(16)?, be(20)?);
    let color_type = *bytes.get(25)?;
    let mut has_alpha = matches!(color_type, 4 | 6);
    let mut at = 8;
    while let (Some(length), Some(kind)) = (be(at), bytes.get(at + 4..at + 8)) {
        match kind {
            b"tRNS" => has_alpha = true,
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        at += 12 + length as usize;
    }
    Some(PngHeader {
        width,
        height,
        has_alpha,
    })
}

#[cfg(test)]
mod tests;
