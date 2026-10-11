//! Where weights, the runtime and outputs live, and pruning what no pin
//! names (IMG-004, IMG-020).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::info;

use crate::args::ModelPaths;
use crate::catalog::{HUB, PinnedFile, WeightSet};
use crate::error::Result;
use crate::fetch::{Fetcher, Pin};

/// The generator's own directory; nothing outside it is written or pruned.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    hub: String,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            hub: HUB.to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_hub(root: PathBuf, hub: String) -> Self {
        Self { root, hub }
    }

    /// The arkavo data directory's image-gen folder: on macOS
    /// ~/Library/Application Support/arkavo/image-gen.
    pub fn default_root() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("arkavo").join("image-gen"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }

    pub fn outputs_dir(&self) -> PathBuf {
        self.root.join("outputs")
    }

    pub fn model_path(&self, file: &PinnedFile) -> PathBuf {
        self.models_dir().join(file.store_name())
    }

    /// Bytes `set` still needs fetched; a file shared by two of its roles
    /// counts once, and a partial download counts what it lacks.
    pub fn missing_bytes(&self, set: &WeightSet) -> u64 {
        distinct(set)
            .into_iter()
            .map(|file| file.size.saturating_sub(self.have(file)))
            .sum()
    }

    fn have(&self, file: &PinnedFile) -> u64 {
        let done = self.model_path(file);
        if std::fs::metadata(&done).is_ok_and(|m| m.len() == file.size) {
            return file.size;
        }
        let mut part = done.into_os_string();
        part.push(".part");
        std::fs::metadata(part).map_or(0, |m| m.len().min(file.size))
    }

    /// Fetch every file of `set`, reporting bytes fetched of bytes needed.
    pub async fn fetch_set(
        &self,
        fetcher: &Fetcher,
        set: &WeightSet,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<ModelPaths> {
        let needed = self.missing_bytes(set);
        let done = AtomicU64::new(0);
        for file in distinct(set) {
            let start = self.have(file);
            let url = file.url_at(&self.hub);
            let pin = Pin {
                url: &url,
                size: file.size,
                sha256: file.sha256,
            };
            let base = done.load(Ordering::Relaxed);
            let report = |written: u64| progress(base + written.saturating_sub(start), needed);
            fetcher.fetch(pin, &self.model_path(file), &report).await?;
            done.fetch_add(file.size - start, Ordering::Relaxed);
        }
        Ok(ModelPaths {
            diffusion: self.model_path(set.diffusion),
            text_encoder: self.model_path(set.text_encoder),
            vae: self.model_path(set.vae),
        })
    }

    /// Delete weights, partial downloads and runtimes no pin of this
    /// release names, and outputs a previous run left behind.
    pub fn prune(&self, keep_models: &BTreeSet<String>, keep_runtime: &str) -> Result<Pruned> {
        let mut pruned = Pruned::default();
        for entry in read_dir(&self.models_dir())? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".part").unwrap_or(&name);
            if !keep_models.contains(stem) {
                pruned.remove(&entry.path(), name)?;
            }
        }
        for entry in read_dir(&self.runtime_dir())? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != keep_runtime && name != format!("{keep_runtime}.zip.part") {
                pruned.remove(&entry.path(), name)?;
            }
        }
        for entry in read_dir(&self.outputs_dir())? {
            pruned.remove(
                &entry.path(),
                entry.file_name().to_string_lossy().into_owned(),
            )?;
        }
        if !pruned.names.is_empty() {
            info!(bytes = pruned.bytes, names = ?pruned.names, "pruned image-gen files no pin names");
        }
        Ok(pruned)
    }
}

/// What a prune removed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    pub names: Vec<String>,
    pub bytes: u64,
}

impl Pruned {
    fn remove(&mut self, path: &Path, name: String) -> Result<()> {
        let meta = std::fs::symlink_metadata(path)?;
        if meta.is_dir() {
            self.bytes += dir_size(path);
            std::fs::remove_dir_all(path)?;
        } else {
            self.bytes += meta.len();
            std::fs::remove_file(path)?;
        }
        self.names.push(name);
        Ok(())
    }
}

fn distinct(set: &WeightSet) -> Vec<&'static PinnedFile> {
    let mut files: Vec<&'static PinnedFile> = Vec::new();
    for file in set.files() {
        if !files.iter().any(|f| f.sha256 == file.sha256) {
            files.push(file);
        }
    }
    files
}

fn read_dir(dir: &Path) -> Result<Vec<std::fs::DirEntry>> {
    match std::fs::read_dir(dir) {
        Ok(entries) => Ok(entries.collect::<std::io::Result<_>>()?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

fn dir_size(dir: &Path) -> u64 {
    read_dir(dir)
        .unwrap_or_default()
        .iter()
        .map(|e| {
            let path = e.path();
            match std::fs::symlink_metadata(&path) {
                Ok(m) if m.is_dir() => dir_size(&path),
                Ok(m) => m.len(),
                Err(_) => 0,
            }
        })
        .sum()
}

#[cfg(test)]
mod tests;
