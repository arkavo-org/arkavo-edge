//! Where weights and the runtime live, and pruning runtimes no pin names
//! (IMG-004, IMG-020).
//!
//! Weights are Hub files, so they go in the Hugging Face cache through
//! hf-hub, beside the chat models and where `hf cache` sees them. The
//! runtime is a GitHub release and the lock files are arkavo's own, so
//! they stay in the generator's directory.

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use hf_hub::api::tokio::{Api, ApiBuilder, Progress as HubProgress};
use hf_hub::{Cache, Repo, RepoType};
use tracing::{info, warn};

use crate::args::ModelPaths;
use crate::catalog::{HUB, PinnedFile, WeightSet};
use crate::error::{ImageGenError, Result};
use crate::fetch::{check_space, sha256_file};

const ATTEMPTS: u32 = 3;
const BACKOFF: Duration = Duration::from_secs(2);
/// Ranges hf-hub fetches at once; each is held in memory until written.
const PARALLEL_RANGES: usize = 8;
const REPORT_EVERY: Duration = Duration::from_millis(250);

/// The weights in the Hugging Face cache and the generator's own directory.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    cache: Cache,
    hub: String,
    /// Hashes this process has checked, so each file is read through once.
    verified: Arc<Mutex<HashSet<&'static str>>>,
}

impl Store {
    /// The runtime under `root`; weights in the Hugging Face cache
    /// (`HF_HOME/hub`, else ~/.cache/huggingface/hub).
    pub fn new(root: PathBuf) -> Self {
        Self::with_cache(root, Cache::from_env())
    }

    pub fn with_cache(root: PathBuf, cache: Cache) -> Self {
        Self {
            root,
            cache,
            hub: HUB.to_string(),
            verified: Arc::default(),
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub(crate) fn with_hub(root: PathBuf, cache: Cache, hub: String) -> Self {
        Self {
            hub,
            ..Self::with_cache(root, cache)
        }
    }

    /// The arkavo data directory's image-gen folder: on macOS
    /// ~/Library/Application Support/arkavo/image-gen.
    pub fn default_root() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("arkavo").join("image-gen"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }

    fn repo(file: &PinnedFile) -> Repo {
        Repo::with_revision(
            file.repo.to_string(),
            RepoType::Model,
            file.revision.to_string(),
        )
    }

    /// The file's link in the snapshot of its pinned commit. Resolved
    /// directly, not through refs/, which `hf download --revision <commit>`
    /// does not write, so a file another Hub client fetched is found.
    pub fn model_path(&self, file: &PinnedFile) -> PathBuf {
        self.cache
            .repo(Self::repo(file))
            .pointer_path(file.revision)
            .join(file.path)
    }

    /// The blob behind the link, named by the Hub's etag, which for these
    /// files is their SHA-256.
    fn blob_path(&self, file: &PinnedFile) -> PathBuf {
        self.cache.repo(Self::repo(file)).blob_path(file.sha256)
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
        if std::fs::metadata(self.model_path(file)).is_ok_and(|m| m.len() == file.size) {
            return file.size;
        }
        partial_bytes(&self.blob_path(file), file.size)
    }

    /// Fetch every file of `set` at its pinned commit and check each against
    /// its pin, reporting bytes fetched of bytes needed.
    pub async fn fetch_set(
        &self,
        set: &WeightSet,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<ModelPaths> {
        let needed = self.missing_bytes(set);
        if needed > 0 {
            tokio::fs::create_dir_all(self.cache.path()).await?;
            check_space(self.cache.path(), needed)?;
        }
        let mut done = 0;
        for file in distinct(set) {
            let start = self.have(file);
            if start < file.size {
                let report = |written: u64| progress(done + written.saturating_sub(start), needed);
                self.download(file, &report).await?;
                done += file.size - start;
            }
            self.verify(file).await?;
        }
        Ok(ModelPaths {
            diffusion: self.model_path(set.diffusion),
            text_encoder: self.model_path(set.text_encoder),
            vae: self.model_path(set.vae),
        })
    }

    /// No token is sent, even one saved in the cache: every pin is public.
    fn api(&self) -> Result<Api> {
        ApiBuilder::from_cache(self.cache.clone())
            .with_endpoint(self.hub.clone())
            .with_token(None)
            .with_progress(false)
            .with_max_files(PARALLEL_RANGES)
            .build()
            .map_err(|e| ImageGenError::Download {
                url: self.hub.clone(),
                reason: e.to_string(),
            })
    }

    /// Download through hf-hub, which resumes from its partial file. It
    /// retries nothing itself, so interrupted attempts are retried here.
    async fn download(
        &self,
        file: &PinnedFile,
        report: &(dyn Fn(u64) + Send + Sync),
    ) -> Result<()> {
        let url = file.url_at(&self.hub);
        let failed = |reason: String| ImageGenError::Download {
            url: url.clone(),
            reason,
        };
        let api = self.api()?;
        self.check_offer(&api, file, &url).await?;
        let repo = api.repo(Self::repo(file));
        let mut last_error = String::new();
        for attempt in 1..=ATTEMPTS {
            let written = Counter::default();
            let mut fetch = std::pin::pin!(repo.download_with_progress(file.path, written.clone()));
            let mut tick = tokio::time::interval(REPORT_EVERY);
            let outcome = loop {
                tokio::select! {
                    outcome = &mut fetch => break outcome,
                    _ = tick.tick() => report(written.get()),
                }
            };
            match outcome {
                Ok(_) => {
                    report(file.size);
                    return Ok(());
                }
                Err(e) => {
                    warn!(url, attempt, error = %e, "download interrupted; resuming");
                    last_error = e.to_string();
                    if attempt < ATTEMPTS {
                        tokio::time::sleep(BACKOFF * attempt).await;
                    }
                }
            }
        }
        Err(failed(last_error))
    }

    /// Refuse before any byte when the Hub offers another file than the
    /// pin: hf-hub would otherwise fetch it all before the hash check.
    async fn check_offer(&self, api: &Api, file: &PinnedFile, url: &str) -> Result<()> {
        let offered = api
            .metadata(url)
            .await
            .map_err(|e| ImageGenError::Download {
                url: url.to_string(),
                reason: e.to_string(),
            })?;
        if offered.size() as u64 != file.size {
            return Err(ImageGenError::SizeMismatch {
                name: url.to_string(),
                expected: file.size,
                actual: offered.size() as u64,
            });
        }
        if offered.etag() != file.sha256 {
            return Err(ImageGenError::HashMismatch {
                name: url.to_string(),
                expected: file.sha256.to_string(),
                actual: offered.etag().to_string(),
            });
        }
        Ok(())
    }

    /// Hash `file` once per process before it is loaded: other Hub clients
    /// write this cache too, and hf-hub checks no hash. A file that does not
    /// match is removed, link and blob, so the next request fetches it again.
    async fn verify(&self, file: &'static PinnedFile) -> Result<()> {
        if lock(&self.verified).contains(file.sha256) {
            return Ok(());
        }
        let link = self.model_path(file);
        let actual = sha256_file(&link).await?;
        if actual != file.sha256 {
            let blob = std::fs::canonicalize(&link);
            std::fs::remove_file(&link)?;
            if let Ok(blob) = blob {
                std::fs::remove_file(blob)?;
            }
            return Err(ImageGenError::HashMismatch {
                name: file.url_at(&self.hub),
                expected: file.sha256.to_string(),
                actual,
            });
        }
        lock(&self.verified).insert(file.sha256);
        info!(path = %link.display(), bytes = file.size, "pinned file verified");
        Ok(())
    }

    /// Delete runtimes no pin of this release names. Weights are left
    /// alone: the Hugging Face cache is shared, and `hf cache` manages it.
    /// Outputs are not here either: each process keeps its own in a
    /// temporary directory, which another agent's start never touches.
    pub fn prune(&self, keep_runtime: &str) -> Result<Pruned> {
        let mut pruned = Pruned::default();
        for entry in read_dir(&self.runtime_dir())? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name != keep_runtime && name != format!("{keep_runtime}.zip.part") {
                pruned.remove(&entry.path(), name)?;
            }
        }
        if !pruned.names.is_empty() {
            info!(bytes = pruned.bytes, names = ?pruned.names, "pruned runtimes no pin names");
        }
        Ok(pruned)
    }
}

/// Bytes hf-hub reports written, which it reports as deltas from its tasks.
#[derive(Debug, Clone, Default)]
struct Counter(Arc<AtomicU64>);

impl Counter {
    fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl HubProgress for Counter {
    async fn init(&mut self, _size: usize, _filename: &str) {}

    async fn update(&mut self, size: usize) {
        self.0.fetch_add(size as u64, Ordering::Relaxed);
    }

    async fn finish(&mut self) {}
}

/// What hf-hub's partial download holds. It preallocates the file one
/// u64 longer than the blob and keeps there the count of bytes written in
/// order, which is where it resumes.
fn partial_bytes(blob: &Path, size: u64) -> u64 {
    let mut part = blob.to_path_buf();
    part.set_extension("sync.part");
    let committed = || -> std::io::Result<u64> {
        let mut file = std::fs::File::open(&part)?;
        if file.metadata()?.len() != size + 8 {
            return Ok(0);
        }
        file.seek(SeekFrom::Start(size))?;
        let mut trailer = [0u8; 8];
        file.read_exact(&mut trailer)?;
        Ok(u64::from_le_bytes(trailer).min(size))
    };
    committed().unwrap_or(0)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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
