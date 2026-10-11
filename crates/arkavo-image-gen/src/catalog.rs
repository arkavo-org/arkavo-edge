//! The models this release can run, and the runtime that runs them.
//!
//! The catalog is compiled in: weights and the runtime are parsed and
//! executed on the host, so a new model or version arrives as a reviewed
//! change to this file and to `specs/arkavo-edge/image-gen.spec.yaml`
//! (IMG-016, IMG-017), never as data fetched at run time.

mod pins;

pub use pins::RUNTIME;

/// Where pinned files are fetched from.
pub const HUB: &str = "https://huggingface.co";

/// A file fetched from Hugging Face, pinned by commit, size and hash.
#[derive(Debug)]
pub struct PinnedFile {
    pub repo: &'static str,
    pub revision: &'static str,
    pub path: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

impl PinnedFile {
    /// The resolve URL at the pinned commit, never a branch.
    pub fn url(&self) -> String {
        self.url_at(HUB)
    }

    /// The same path on another host serving the Hub's layout.
    pub fn url_at(&self, hub: &str) -> String {
        format!(
            "{hub}/{}/resolve/{}/{}",
            self.repo, self.revision, self.path
        )
    }

    /// The name the file is stored under. Keyed by hash so a file several
    /// entries use is stored once, keeping the upstream name because
    /// stable-diffusion.cpp picks a loader by extension.
    pub fn store_name(&self) -> String {
        let base = self.path.rsplit('/').next().unwrap_or(self.path);
        format!("{}-{base}", self.sha256)
    }
}

/// The stable-diffusion.cpp release the generator runs.
#[derive(Debug, Clone, Copy)]
pub struct RuntimePin {
    /// Release build number; entries name the oldest build they run on.
    pub build: u32,
    pub tag: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
    /// Archive members that are extracted; anything else fails the install.
    pub members: &'static [&'static str],
    /// The member that is executed.
    pub cli: &'static str,
}

/// Size class of a weight set, largest first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum SetLabel {
    Xl,
    L,
    M,
    S,
}

/// The three files one run of an entry loads.
#[derive(Debug)]
pub struct WeightSet {
    pub label: SetLabel,
    pub diffusion: &'static PinnedFile,
    pub text_encoder: &'static PinnedFile,
    pub vae: &'static PinnedFile,
}

impl WeightSet {
    pub fn files(&self) -> [&'static PinnedFile; 3] {
        [self.diffusion, self.text_encoder, self.vae]
    }

    pub fn bytes(&self) -> u64 {
        self.files().iter().map(|f| f.size).sum()
    }
}

/// How an entry is sampled.
#[derive(Debug)]
pub struct Profile {
    pub steps: u32,
    pub sampler: &'static str,
    /// A distilled schedule passed as --sigmas; `None` lets the runtime
    /// choose the schedule the model's scheduler config asks for.
    pub sigmas: Option<&'static str>,
    /// Whether true CFG with a negative prompt is offered, and its ceiling.
    pub max_guidance: Option<f32>,
}

/// Output sizes an entry accepts.
#[derive(Debug)]
pub struct Limits {
    pub step: u32,
    pub min_side: u32,
    pub max_side: u32,
    pub max_pixels: u64,
    pub default_width: u32,
    pub default_height: u32,
}

/// Memory a run needs above its weights, measured at one size (IMG-018).
#[derive(Debug, Clone, Copy)]
pub struct Allowance {
    pub pixels: u64,
    /// Measured with true CFG on, which runs both conditions each step.
    pub guided: bool,
    pub bytes: u64,
}

/// One model the catalog offers.
#[derive(Debug)]
pub struct Entry {
    pub id: &'static str,
    pub model_repo: &'static str,
    pub model_revision: &'static str,
    /// One line for the tool schema: what the entry is for.
    pub summary: &'static str,
    pub license: &'static str,
    pub license_url: &'static str,
    pub profile: Profile,
    pub limits: Limits,
    /// Prompt framing that asks the model for an alpha channel.
    pub rgba_template: Option<(&'static str, &'static str)>,
    pub min_runtime_build: u32,
    /// Largest first.
    pub sets: &'static [WeightSet],
    pub allowances: &'static [Allowance],
}

impl Entry {
    /// The allowance for a run at `pixels`: the smallest measured point at
    /// least that large for the same guidance, `None` when nothing covers it.
    pub fn allowance(&self, pixels: u64, guided: bool) -> Option<u64> {
        self.allowances
            .iter()
            .filter(|a| a.guided == guided && a.pixels >= pixels)
            .min_by_key(|a| a.pixels)
            .map(|a| a.bytes)
    }

    /// The largest pixel count any measured point covers for `guided`.
    pub fn largest_measured(&self, guided: bool) -> Option<u64> {
        self.allowances
            .iter()
            .filter(|a| a.guided == guided)
            .map(|a| a.pixels)
            .max()
    }
}

/// Every entry this release offers.
pub fn entries() -> &'static [Entry] {
    pins::ENTRIES
}

/// Aliases and the entry ids they name. Moving one is an explicit change.
pub const ALIASES: &[(&str, &str)] = &[
    ("qwen-image", "qwen-image-2.1-turbo"),
    ("qwen-image-quality", "qwen-image-2.1"),
];

/// What a request without a model gets.
pub const DEFAULT_MODEL: &str = "qwen-image";

/// An entry and the alias that named it, if any.
#[derive(Debug, Clone, Copy)]
pub struct Resolved {
    pub entry: &'static Entry,
    pub alias: Option<&'static str>,
}

/// Resolve a catalog id or alias.
pub fn resolve(name: &str) -> Option<Resolved> {
    if let Some(entry) = entries().iter().find(|e| e.id == name) {
        return Some(Resolved { entry, alias: None });
    }
    let (alias, id) = ALIASES.iter().find(|(alias, _)| *alias == name)?;
    let entry = entries().iter().find(|e| e.id == *id)?;
    Some(Resolved {
        entry,
        alias: Some(alias),
    })
}

/// Every name a request may use: entry ids, then aliases.
pub fn names() -> Vec<&'static str> {
    entries()
        .iter()
        .map(|e| e.id)
        .chain(ALIASES.iter().map(|(alias, _)| *alias))
        .collect()
}

/// Every file the catalog and the runtime pin name, by store name.
pub fn pinned_store_names() -> std::collections::BTreeSet<String> {
    entries()
        .iter()
        .flat_map(|e| e.sets.iter())
        .flat_map(WeightSet::files)
        .map(PinnedFile::store_name)
        .collect()
}

#[cfg(test)]
mod tests;
