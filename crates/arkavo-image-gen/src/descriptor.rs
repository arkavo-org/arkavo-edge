//! What a capable member reports, and how a requester picks among members
//! (ASSET-001, ASSET-002).

use serde::{Deserialize, Serialize};

use crate::catalog::{self, SetLabel};
use crate::request::ImageRequest;

/// The capability name members report.
pub const CAPABILITY: &str = "image_generation";

/// A2A data part schema of an asset request.
pub const ASSET_REQUEST_SCHEMA: &str = "urn:arkavo:asset:request:v1";

/// A2A data part schema of an asset manifest with references.
pub const ASSET_MANIFEST_SCHEMA: &str = "urn:arkavo:asset:manifest:v1";

/// A member's image generation capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Descriptor {
    pub chip: Option<String>,
    pub unified_memory_gib: f64,
    pub runtime_build: u32,
    pub queue_depth: usize,
    pub models: Vec<ModelRow>,
}

/// One catalog entry a member offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRow {
    pub id: String,
    pub aliases: Vec<String>,
    pub weight_set: SetLabel,
    pub ready: bool,
    pub bytes_remaining: u64,
    pub largest_pixels: u64,
    pub largest_pixels_guided: Option<u64>,
    pub steps: u32,
    pub seconds_per_step: Option<f64>,
}

/// What a request needs from a member, as far as the requester can tell.
#[derive(Debug, Clone, PartialEq)]
pub struct Need {
    pub model: String,
    /// `None` when the requester's catalog does not know the model, so the
    /// member checks the size itself.
    pub pixels: Option<u64>,
    pub guided: bool,
    pub images: u32,
}

impl Need {
    pub fn of(request: &ImageRequest) -> Self {
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| catalog::DEFAULT_MODEL.to_string());
        match request.validate() {
            Ok(job) => Self {
                model,
                pixels: Some(job.pixels()),
                guided: job.guided(),
                images: job.count,
            },
            Err(_) => Self {
                pixels: request
                    .width
                    .zip(request.height)
                    .map(|(w, h)| u64::from(w) * u64::from(h)),
                guided: request.negative_prompt.is_some(),
                images: request.count.unwrap_or(1),
                model,
            },
        }
    }
}

impl Descriptor {
    /// The row that serves `need`: named by id or alias, and large enough.
    pub fn row(&self, need: &Need) -> Option<&ModelRow> {
        self.models.iter().find(|row| {
            let named = row.id == need.model || row.aliases.contains(&need.model);
            let largest = if need.guided {
                row.largest_pixels_guided
            } else {
                Some(row.largest_pixels)
            };
            named && need.pixels.is_none_or(|p| largest.is_some_and(|l| l >= p))
        })
    }

    /// Estimated seconds until the request is done here, counting the
    /// queue; `None` until the member has measured its speed.
    pub fn estimate(&self, need: &Need) -> Option<f64> {
        let row = self.row(need)?;
        let megapixels = need.pixels.unwrap_or(1 << 20) as f64 / f64::from(1u32 << 20);
        let per_image = f64::from(row.steps) * row.seconds_per_step? * megapixels;
        Some(per_image * (f64::from(need.images) + self.queue_depth as f64))
    }
}

/// Members that can serve `need`, best first: ready, then the larger weight
/// set, then the shorter estimate (unmeasured last), then the shorter queue.
pub fn rank<K>(members: Vec<(K, Descriptor)>, need: &Need) -> Vec<K> {
    let mut eligible: Vec<_> = members
        .into_iter()
        .filter_map(|(key, d)| {
            let row = d.row(need)?;
            let order = (!row.ready, row.weight_set, d.estimate(need), d.queue_depth);
            Some((order, key))
        })
        .collect();
    eligible.sort_by(|(a, _), (b, _)| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(match (a.2, b.2) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then(a.3.cmp(&b.3))
    });
    eligible.into_iter().map(|(_, key)| key).collect()
}

#[cfg(test)]
mod tests;
