//! What a requester asks for, and the validated job it becomes (IMG-007,
//! IMG-009, IMG-019).

use serde::{Deserialize, Serialize};

use crate::catalog::{self, Entry};
use crate::error::{ImageGenError, Result};

const MAX_PROMPT_CHARS: usize = 2_000;
const MAX_NAME_CHARS: usize = 64;
const MAX_COUNT: u32 = 4;

/// A generation request as it arrives from a tool call or an A2A message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageRequest {
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tileable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    /// Editing input; recognized so it can be refused by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_images: Option<serde_json::Value>,
}

/// A request checked against its catalog entry, with every default filled.
#[derive(Debug, Clone)]
pub struct Job {
    pub entry: &'static Entry,
    pub alias: Option<&'static str>,
    pub prompt: String,
    pub width: u32,
    pub height: u32,
    pub count: u32,
    pub seed: u32,
    pub name: String,
    pub transparent: bool,
    pub tileable: bool,
    pub guidance: f32,
    pub negative_prompt: Option<String>,
}

impl Job {
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Whether true CFG runs, which needs both guidance and a negative prompt.
    pub fn guided(&self) -> bool {
        self.negative_prompt.is_some()
    }

    /// The prompt the model sees: the entry's RGBA framing when transparent.
    pub fn effective_prompt(&self) -> String {
        match (self.transparent, self.entry.rgba_template) {
            (true, Some((prefix, suffix))) => {
                format!("{prefix}{}{suffix}", self.prompt.trim_end_matches('.'))
            }
            _ => self.prompt.clone(),
        }
    }
}

fn invalid(field: &'static str, reason: impl Into<String>) -> ImageGenError {
    ImageGenError::InvalidParams {
        field,
        reason: reason.into(),
    }
}

impl ImageRequest {
    /// Validate against the catalog; nothing is fetched or started for a
    /// request that fails here.
    pub fn validate(&self) -> Result<Job> {
        if self.reference_images.is_some() {
            return Err(ImageGenError::UnsupportedFeature {
                feature: "reference_images",
                reason: "image editing is not available in this release".to_string(),
            });
        }
        let name = self.model.as_deref().unwrap_or(catalog::DEFAULT_MODEL);
        let resolved = catalog::resolve(name).ok_or_else(|| ImageGenError::UnknownModel {
            name: name.to_string(),
            known: catalog::names().join(", "),
        })?;
        let entry = resolved.entry;

        let prompt = self.prompt.trim();
        let chars = prompt.chars().count();
        if chars == 0 || chars > MAX_PROMPT_CHARS {
            return Err(invalid(
                "prompt",
                format!("must be 1 to {MAX_PROMPT_CHARS} characters, got {chars}"),
            ));
        }

        let width = self.width.unwrap_or(entry.limits.default_width);
        let height = self.height.unwrap_or(entry.limits.default_height);
        check_side(entry, "width", width)?;
        check_side(entry, "height", height)?;
        let pixels = u64::from(width) * u64::from(height);
        if pixels > entry.limits.max_pixels {
            return Err(invalid(
                "width",
                format!(
                    "{width}x{height} is {pixels} pixels; {} allows at most {}",
                    entry.id, entry.limits.max_pixels
                ),
            ));
        }

        let count = self.count.unwrap_or(1);
        if !(1..=MAX_COUNT).contains(&count) {
            return Err(invalid("count", format!("must be 1 to {MAX_COUNT}")));
        }

        let name_value = self.name.clone().unwrap_or_else(|| "asset".to_string());
        if name_value.is_empty()
            || name_value.chars().count() > MAX_NAME_CHARS
            || !name_value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(invalid(
                "name",
                format!("must be 1 to {MAX_NAME_CHARS} characters from A-Z, a-z, 0-9, _ and -"),
            ));
        }

        let transparent = self.transparent.unwrap_or(false);
        if transparent && entry.rgba_template.is_none() {
            return Err(invalid(
                "transparent",
                format!("{} has no transparent output", entry.id),
            ));
        }

        let (guidance, negative_prompt) = self.guidance_for(entry)?;

        Ok(Job {
            entry,
            alias: resolved.alias,
            prompt: prompt.to_string(),
            width,
            height,
            count,
            seed: self.seed.unwrap_or_else(rand::random),
            name: name_value,
            transparent,
            tileable: self.tileable.unwrap_or(false),
            guidance,
            negative_prompt,
        })
    }

    /// Guidance and a negative prompt come together or not at all: true CFG
    /// needs both, and either alone would be silently ignored.
    fn guidance_for(&self, entry: &Entry) -> Result<(f32, Option<String>)> {
        let guidance = self.guidance.unwrap_or(1.0);
        let negative = self
            .negative_prompt
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        let Some(max) = entry.profile.max_guidance else {
            if negative.is_some() {
                return Err(invalid(
                    "negative_prompt",
                    format!(
                        "{} samples at CFG 1, where a negative prompt has no effect",
                        entry.id
                    ),
                ));
            }
            if (guidance - 1.0).abs() > f32::EPSILON {
                return Err(invalid(
                    "guidance",
                    format!("{} samples at CFG 1", entry.id),
                ));
            }
            return Ok((1.0, None));
        };
        if !(1.0..=max).contains(&guidance) {
            return Err(invalid("guidance", format!("must be 1 to {max}")));
        }
        match (guidance > 1.0, negative) {
            (true, Some(n)) => {
                if n.chars().count() > MAX_PROMPT_CHARS {
                    return Err(invalid(
                        "negative_prompt",
                        format!("must be at most {MAX_PROMPT_CHARS} characters"),
                    ));
                }
                Ok((guidance, Some(n.to_string())))
            }
            (false, None) => Ok((1.0, None)),
            (true, None) => Err(invalid(
                "negative_prompt",
                "guidance above 1 needs a negative prompt",
            )),
            (false, Some(_)) => Err(invalid(
                "guidance",
                "a negative prompt needs guidance above 1",
            )),
        }
    }
}

fn check_side(entry: &Entry, field: &'static str, side: u32) -> Result<()> {
    let l = &entry.limits;
    if !side.is_multiple_of(l.step) || !(l.min_side..=l.max_side).contains(&side) {
        return Err(invalid(
            field,
            format!(
                "{side} must be a multiple of {} from {} to {}",
                l.step, l.min_side, l.max_side
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
