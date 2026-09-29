//! Locates model weights in the HuggingFace cache and downloads missing ones.

use std::path::{Path, PathBuf};

/// Extension identifying a KAS-protected model (`gguf-tdf/1`).
const PROTECTED_EXTENSION: &str = ".gguf.tdf";

fn is_protected_gguf(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_lowercase().ends_with(PROTECTED_EXTENSION))
}

fn is_plaintext_gguf(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gguf"))
}

/// Resolves a GGUF path, keeping plaintext when it is present.
///
/// Wrapping is additive (`model.gguf` next to `model.gguf.tdf`). Production
/// load still uses `LlamaModel::from_file`, so a sibling TDF must not displace
/// a loadable plaintext file. Fall back to `.gguf.tdf` only when the plaintext
/// path is missing (for example after `--delete-source`).
pub fn resolve_gguf_path(path: &Path) -> PathBuf {
    let name = path.to_string_lossy();
    if name.to_lowercase().ends_with(PROTECTED_EXTENSION) {
        return path.to_path_buf();
    }
    if path.exists() {
        return path.to_path_buf();
    }
    // Build the sibling from the OsStr so a non-UTF-8 path still names the
    // real file on disk.
    let mut protected = path.as_os_str().to_os_string();
    protected.push(".tdf");
    let protected = PathBuf::from(protected);
    if name.to_lowercase().ends_with(".gguf") && protected.exists() {
        tracing::info!(
            "plaintext {:?} is absent; using protected sibling {:?}",
            path,
            protected
        );
        return protected;
    }
    path.to_path_buf()
}

/// Filename prefixes of GGUF files published beside a model's weights that
/// are not themselves a model: vision projectors and drafter heads. They carry
/// the `.gguf` extension, so a scan that goes by extension alone hands one to
/// the loader as if it were the model.
const COMPANION_PREFIXES: [&str; 3] = ["mmproj", "mtp-", "dflash-"];

fn is_companion_gguf(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_lowercase)
        .is_some_and(|n| COMPANION_PREFIXES.iter().any(|p| n.starts_with(p)))
}

/// The command that fetches exactly this file, as shown to the user.
pub fn download_command(repo_id: &str, filename: &str) -> String {
    format!("hf download {repo_id} {filename}")
}

/// Resolve the weights for one specific model, downloading them if needed.
///
/// A caller that names a repo and file gets that model or an error, never
/// another model that happens to be cached: a substitute runs under the
/// requested model's name, memory budget and sampling settings, none of which
/// fit it. Callers that want a small model for their own use, whichever one
/// is on disk, use [`find_small_gguf`] instead.
///
/// Order: the exact file in the cache, then a download, then another build of
/// the same repo that is already cached (a different quantization of the
/// model that was asked for).
///
/// # Errors
/// Names the reason the download failed and the command that fetches the file.
pub async fn find_gguf_model(repo_id: &str, filename: &str) -> Result<PathBuf, String> {
    let cache = get_hf_cache_dir();
    resolve_gguf_model(cache.as_deref(), repo_id, filename, || {
        download_from_hub(repo_id, filename)
    })
    .await
}

/// [`find_gguf_model`] with the cache location and the download step supplied
/// by the caller, so resolution can be tested without the network or `HF_HOME`.
async fn resolve_gguf_model<F, Fut>(
    cache: Option<&Path>,
    repo_id: &str,
    filename: &str,
    download: F,
) -> Result<PathBuf, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<PathBuf, String>>,
{
    let snapshots = cache.map(|c| repo_snapshots_dir(c, repo_id));

    // The cache is checked first so an already-downloaded model loads with no
    // network round trip.
    if let Some(path) = snapshots
        .as_deref()
        .and_then(|dir| find_file_in_dir(dir, filename))
    {
        tracing::debug!(repo_id, filename, path = %path.display(), "model found in cache");
        return Ok(resolve_gguf_path(&path));
    }

    let reason = match download().await {
        Ok(path) => {
            tracing::debug!(repo_id, filename, path = %path.display(), "model downloaded");
            return Ok(resolve_gguf_path(&path));
        }
        Err(reason) => reason,
    };
    tracing::warn!(repo_id, filename, %reason, "model download failed");

    if let Some(path) = snapshots.as_deref().and_then(find_gguf_in_dir) {
        tracing::warn!(
            repo_id,
            requested = filename,
            using = %path.display(),
            "requested file is not cached; using another build from the same repository"
        );
        return Ok(resolve_gguf_path(&path));
    }

    Err(format!(
        "Model {repo_id}/{filename} is not cached and could not be downloaded: {reason}. \
         Download with: {}",
        download_command(repo_id, filename)
    ))
}

async fn download_from_hub(repo_id: &str, filename: &str) -> Result<PathBuf, String> {
    let api = hf_hub::api::tokio::Api::new()
        .map_err(|e| format!("failed to initialize the HuggingFace API: {e}"))?;
    api.repo(hf_hub::Repo::model(repo_id.to_string()))
        .get(filename)
        .await
        .map_err(|e| e.to_string())
}

/// Snapshot directory of a repo in the cache: "org/model" is stored under
/// "models--org--model".
fn repo_snapshots_dir(cache: &Path, repo_id: &str) -> PathBuf {
    cache
        .join(format!("models--{}", repo_id.replace('/', "--")))
        .join("snapshots")
}

/// Get the HuggingFace cache directory
fn get_hf_cache_dir() -> Option<PathBuf> {
    // Check HF_HOME environment variable first
    if let Ok(hf_home) = std::env::var("HF_HOME") {
        return Some(PathBuf::from(hf_home).join("hub"));
    }

    // Fall back to default location: ~/.cache/huggingface/hub
    dirs::home_dir().map(|home| home.join(".cache").join("huggingface").join("hub"))
}

/// Recursively find a GGUF artifact, preferring plaintext over `.gguf.tdf`.
///
/// One pass over the tree: the first plaintext `.gguf` wins immediately; the
/// first `.gguf.tdf` seen is kept as the fallback. A large HF cache is not
/// walked twice.
fn find_gguf_in_dir(dir: &std::path::Path) -> Option<PathBuf> {
    let mut protected = None;
    find_artifact_in_dir(dir, &mut protected).or(protected)
}

/// Returns the first plaintext GGUF under `dir`; records the first protected
/// artifact in `protected` when no plaintext has been found yet. Companion
/// files are passed over: they are not a model in either form.
fn find_artifact_in_dir(dir: &std::path::Path, protected: &mut Option<PathBuf>) -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_file() {
                if is_companion_gguf(&path) {
                    continue;
                }
                if is_plaintext_gguf(&path) {
                    return Some(path);
                }
                if protected.is_none() && is_protected_gguf(&path) {
                    *protected = Some(path);
                }
            } else if path.is_dir()
                && let Some(found) = find_artifact_in_dir(&path, protected)
            {
                return Some(found);
            }
        }
    }
    None
}

/// Check if a specific model exists in the HuggingFace cache (no download, no fallback)
///
/// Returns true if the model file is already cached, false otherwise.
/// Used by `is_model_available` to check if a model is cached locally.
pub fn is_model_cached(repo_id: &str, filename: &str) -> bool {
    get_hf_cache_dir().is_some_and(|cache| {
        find_file_in_dir(&repo_snapshots_dir(&cache, repo_id), filename).is_some()
    })
}

/// Find a specific file in a directory tree
fn find_file_in_dir(dir: &std::path::Path, filename: &str) -> Option<PathBuf> {
    if let Some(found) = find_named_in_dir(dir, filename) {
        return Some(found);
    }
    let lower = filename.to_lowercase();
    if lower.ends_with(".gguf") && !lower.ends_with(PROTECTED_EXTENSION) {
        let protected_name = format!("{filename}.tdf");
        return find_named_in_dir(dir, &protected_name);
    }
    None
}

fn find_named_in_dir(dir: &std::path::Path, filename: &str) -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.file_name().and_then(|s| s.to_str()) == Some(filename) {
                return Some(path);
            } else if path.is_dir()
                && let Some(found) = find_named_in_dir(&path, filename)
            {
                return Some(found);
            }
        }
    }
    None
}

/// Find the mmproj (vision projector) file for a given model GGUF path.
///
/// Scans the parent directory of the resolved model path for files matching
/// `mmproj*.gguf`. When multiple quant variants exist, prefers the smallest
/// (F16 over BF16/F32) to minimize memory overhead.
pub fn find_mmproj_for_model(model_path: &std::path::Path) -> Option<PathBuf> {
    let parent = model_path.parent()?;
    let entries = std::fs::read_dir(parent).ok()?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
            && name.starts_with("mmproj")
            && name.ends_with(".gguf")
        {
            candidates.push(path);
        }
    }
    // Prefer smallest quant: Q4 > Q8 > F16 > BF16 > F32
    candidates.sort_by_key(|p| {
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_uppercase();
        if name.contains("Q4") {
            0
        } else if name.contains("Q8") {
            1
        } else if name.contains("F16") && !name.contains("BF16") {
            2
        } else if name.contains("BF16") {
            3
        } else {
            4
        }
    });
    if let Some(best) = candidates.first() {
        tracing::info!("Found mmproj for vision support: {}", best.display());
        return Some(best.clone());
    }
    None
}

/// Largest model file the classifier and judge will load, exclusive.
///
/// They answer a one-line question on every routed request, in every agent
/// process, so they get the fast tier only. 5 GB is the boundary the server
/// already uses between its fast and medium speed tiers
/// (`ModelChoice::size_bytes`): it admits the edge models an install ships
/// for this purpose (Qwen3.5 0.8B, Gemma 4 E2B and E4B) and keeps out the
/// 8B-and-up role models, whose context alone costs hundreds of MB per call.
pub const SMALL_MODEL_SIZE_CEILING: u64 = 5_000_000_000;

/// Substrings of a GGUF filename that mark a model that cannot answer a
/// prompt: embedders and rerankers produce vectors and scores, not text.
const NON_GENERATIVE_MARKERS: [&str; 2] = ["embed", "rerank"];

/// Why no model was picked for classification and judging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoSmallModel {
    /// The cache holds no plaintext model at all.
    NoneCached,
    /// The cache holds models, and the smallest is still over the ceiling.
    AllTooLarge { smallest: PathBuf, size_bytes: u64 },
}

impl std::fmt::Display for NoSmallModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoneCached => write!(f, "no plaintext GGUF model is cached"),
            Self::AllTooLarge {
                smallest,
                size_bytes,
            } => write!(
                f,
                "the smallest cached model, {}, is {:.1} GB; models of {:.0} GB and over are \
                 not loaded for classification",
                smallest
                    .file_name()
                    .map_or_else(|| smallest.to_string_lossy(), |n| n.to_string_lossy()),
                gigabytes(*size_bytes),
                gigabytes(SMALL_MODEL_SIZE_CEILING),
            ),
        }
    }
}

fn gigabytes(bytes: u64) -> f64 {
    bytes as f64 / 1e9
}

/// Pick the model the routing classifier and response judge load: the
/// smallest **plaintext** `.gguf` in the HuggingFace cache that is under
/// [`SMALL_MODEL_SIZE_CEILING`].
///
/// They construct `LlamaCppProvider` synchronously and cannot rewrap a
/// protected model, so a `.gguf.tdf` is never a candidate here. When nothing
/// qualifies the caller falls back to rule-based classification (or skips
/// judging) instead of loading a role model or failing router init.
///
/// # Errors
/// Says whether the cache is empty or holds only models that are too large.
pub async fn find_small_gguf() -> Result<PathBuf, NoSmallModel> {
    let cache = get_hf_cache_dir().ok_or(NoSmallModel::NoneCached)?;
    find_small_plain_gguf_in(&cache, SMALL_MODEL_SIZE_CEILING)
}

fn find_small_plain_gguf_in(cache: &Path, ceiling: u64) -> Result<PathBuf, NoSmallModel> {
    use crate::decision::ModelChoice;

    // Known-good small chat models go first, so an unfamiliar file that
    // happens to be a little smaller does not displace one that is known to
    // follow the classification prompt.
    let preferred = [ModelChoice::LocalQwen3, ModelChoice::LocalMinistral3B]
        .iter()
        .filter_map(ModelChoice::cache_dir_name)
        .find_map(|repo| smallest_in(&plain_models_in(&cache.join(repo)), ceiling));
    if let Some(found) = preferred {
        return Ok(found);
    }

    let repos = std::fs::read_dir(cache)
        .map_err(|_| NoSmallModel::NoneCached)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("models--"))
        });
    let models: Vec<(u64, PathBuf)> = repos.flat_map(|repo| plain_models_in(&repo)).collect();

    smallest_in(&models, ceiling).ok_or_else(|| {
        smallest_in(&models, u64::MAX).map_or(NoSmallModel::NoneCached, |smallest| {
            let size_bytes = file_size(&smallest);
            NoSmallModel::AllTooLarge {
                smallest,
                size_bytes,
            }
        })
    })
}

/// The smallest model under `ceiling`. Equal sizes fall back to path order so
/// the choice does not depend on directory iteration order.
fn smallest_in(models: &[(u64, PathBuf)], ceiling: u64) -> Option<PathBuf> {
    models
        .iter()
        .filter(|(size, _)| *size < ceiling)
        .min()
        .map(|(_, path)| path.clone())
}

/// Size of the file a path resolves to. HuggingFace snapshots are symlinks
/// into `blobs/`, and `metadata` follows them. An unreadable file sorts last.
fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(u64::MAX, |m| m.len())
}

/// Every plaintext model under `dir` with its size: no `.gguf.tdf`, no
/// companion files, no embedders.
fn plain_models_in(dir: &Path) -> Vec<(u64, PathBuf)> {
    let mut found = Vec::new();
    collect_plain_models(dir, &mut found);
    found
}

fn collect_plain_models(dir: &Path, found: &mut Vec<(u64, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        if path.is_dir() {
            collect_plain_models(&path, found);
        } else if path.is_file()
            && is_plaintext_gguf(&path)
            && !is_companion_gguf(&path)
            && !is_non_generative_gguf(&path)
        {
            found.push((file_size(&path), path));
        }
    }
}

fn is_non_generative_gguf(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_lowercase)
        .is_some_and(|n| NON_GENERATIVE_MARKERS.iter().any(|m| n.contains(m)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("ROUTER-006")]
    #[test]
    fn test_find_mmproj_for_model() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("Qwen3.5-27B-UD-Q6_K_XL.gguf");
        let mmproj = dir.path().join("mmproj-Qwen2.5-VL-7B-f16.gguf");
        std::fs::write(&model, b"model").unwrap();
        std::fs::write(&mmproj, b"mmproj").unwrap();

        let result = find_mmproj_for_model(&model);
        assert!(result.is_some());
        assert!(
            result
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("mmproj")
        );
    }

    #[spec("ROUTER-006")]
    #[test]
    fn test_find_mmproj_none_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("model.gguf");
        std::fs::write(&model, b"model").unwrap();

        assert!(find_mmproj_for_model(&model).is_none());
    }
}

#[cfg(test)]
mod protected_model_tests {
    use super::*;

    /// Wrapping is additive: a sibling `.gguf.tdf` must not displace a
    /// loadable plaintext GGUF. Production load still uses `from_file`.
    #[test]
    fn keeps_the_plaintext_when_both_exist() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("model.gguf");
        let protected = dir.path().join("model.gguf.tdf");
        std::fs::write(&plain, b"GGUF").unwrap();
        std::fs::write(&protected, b"PK\x03\x04").unwrap();

        assert_eq!(resolve_gguf_path(&plain), plain);
    }

    #[test]
    fn keeps_the_plaintext_when_no_protected_sibling_exists() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("model.gguf");
        std::fs::write(&plain, b"GGUF").unwrap();

        assert_eq!(resolve_gguf_path(&plain), plain);
    }

    #[test]
    fn falls_back_to_the_protected_artifact_when_plaintext_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("model.gguf");
        let protected = dir.path().join("model.gguf.tdf");
        std::fs::write(&protected, b"PK\x03\x04").unwrap();

        assert_eq!(resolve_gguf_path(&plain), protected);
    }

    #[test]
    fn a_protected_path_is_returned_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let protected = dir.path().join("model.gguf.tdf");
        std::fs::write(&protected, b"PK\x03\x04").unwrap();

        assert_eq!(resolve_gguf_path(&protected), protected);
        assert!(!dir.path().join("model.gguf.tdf.tdf").exists());
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("Model.GGUF");
        let protected = dir.path().join("Model.GGUF.tdf");
        std::fs::write(&plain, b"GGUF").unwrap();
        std::fs::write(&protected, b"PK\x03\x04").unwrap();

        assert_eq!(resolve_gguf_path(&plain), plain);
        std::fs::remove_file(&plain).unwrap();
        assert_eq!(resolve_gguf_path(&plain), protected);
    }

    #[test]
    fn a_non_gguf_path_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, b"hello").unwrap();

        assert_eq!(resolve_gguf_path(&other), other);
    }

    #[test]
    fn find_gguf_in_dir_keeps_plaintext_when_both_exist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model.gguf"), b"GGUF").unwrap();
        std::fs::write(dir.path().join("model.gguf.tdf"), b"PK\x03\x04").unwrap();

        let found = find_gguf_in_dir(dir.path()).expect("plaintext GGUF must be found");
        assert_eq!(found.file_name().unwrap(), "model.gguf");
    }

    #[test]
    fn find_gguf_in_dir_finds_protected_when_that_is_all_there_is() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model.gguf.tdf"), b"PK\x03\x04").unwrap();

        let found = find_gguf_in_dir(dir.path()).expect("protected artifact must be found");
        assert_eq!(found.file_name().unwrap(), "model.gguf.tdf");
    }

    #[test]
    fn find_file_in_dir_falls_back_to_the_protected_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("model.gguf.tdf"), b"PK\x03\x04").unwrap();

        let found = find_file_in_dir(dir.path(), "model.gguf").expect("tdf sibling");
        assert_eq!(found.file_name().unwrap(), "model.gguf.tdf");
    }

    /// The classifier/judge load synchronously and cannot rewrap: a cache
    /// holding only protected models must yield nothing, not a `.gguf.tdf`.
    #[test]
    fn find_any_gguf_ignores_protected_models() {
        let cache = tempfile::tempdir().unwrap();
        let repo = cache
            .path()
            .join("models--unsloth--Qwen3.5-0.8B-GGUF/snapshots/x");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("Qwen3.5-0.8B-Q4_K_M.gguf.tdf"), b"PK\x03\x04").unwrap();

        assert_eq!(
            find_small_plain_gguf_in(cache.path(), SMALL_MODEL_SIZE_CEILING),
            Err(NoSmallModel::NoneCached)
        );
    }

    #[test]
    fn find_any_gguf_prefers_a_plaintext_qwen_over_other_plaintext_models() {
        let cache = tempfile::tempdir().unwrap();
        let other = cache.path().join("models--org--other/snapshots/x");
        let qwen = cache
            .path()
            .join("models--unsloth--Qwen3.5-0.8B-GGUF/snapshots/x");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(&qwen).unwrap();
        std::fs::write(other.join("other.gguf"), b"GGUF").unwrap();
        std::fs::write(qwen.join("Qwen3.5-0.8B-Q4_K_M.gguf"), b"GGUF").unwrap();
        // A protected sibling next to the preferred plaintext changes nothing.
        std::fs::write(qwen.join("Qwen3.5-0.8B-Q4_K_M.gguf.tdf"), b"PK\x03\x04").unwrap();

        let found = find_small_plain_gguf_in(cache.path(), SMALL_MODEL_SIZE_CEILING).unwrap();
        assert_eq!(found.file_name().unwrap(), "Qwen3.5-0.8B-Q4_K_M.gguf");
    }

    #[test]
    fn find_any_gguf_falls_back_to_any_plaintext_repo_when_preferred_ones_are_protected() {
        let cache = tempfile::tempdir().unwrap();
        let other = cache.path().join("models--org--other/snapshots/x");
        let qwen = cache
            .path()
            .join("models--unsloth--Qwen3.5-0.8B-GGUF/snapshots/x");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::create_dir_all(&qwen).unwrap();
        std::fs::write(other.join("other.gguf"), b"GGUF").unwrap();
        std::fs::write(qwen.join("Qwen3.5-0.8B-Q4_K_M.gguf.tdf"), b"PK\x03\x04").unwrap();

        let found = find_small_plain_gguf_in(cache.path(), SMALL_MODEL_SIZE_CEILING).unwrap();
        assert_eq!(found.file_name().unwrap(), "other.gguf");
    }

    /// Precedence is tree-wide, not per directory: a protected artifact seen
    /// first must not shadow a plaintext GGUF found later in a sibling dir.
    #[test]
    fn plaintext_in_a_later_directory_beats_protected_seen_earlier() {
        let root = tempfile::tempdir().unwrap();
        let a = root.path().join("a-protected");
        let b = root.path().join("b-plain");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("model.gguf.tdf"), b"PK\x03\x04").unwrap();
        std::fs::write(b.join("model.gguf"), b"GGUF").unwrap();

        let found = find_gguf_in_dir(root.path()).expect("plaintext must be found");
        assert_eq!(found, b.join("model.gguf"));
    }

    /// And the fallback still works when the protected file is the only one,
    /// nested deeper than the directory scanned.
    #[test]
    fn protected_in_a_nested_directory_is_found_when_nothing_plain_exists() {
        let root = tempfile::tempdir().unwrap();
        let deep = root.path().join("a/snapshots/x");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("model.gguf.tdf"), b"PK\x03\x04").unwrap();

        let found = find_gguf_in_dir(root.path()).expect("protected must be found");
        assert_eq!(found, deep.join("model.gguf.tdf"));
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod resolution_tests {
    use super::*;
    use arkavo_test_macros::spec;

    const GEMMA_REPO: &str = "ggml-org/gemma-4-12B-it-GGUF";
    const GEMMA_FILE: &str = "gemma-4-12B-it-Q4_0.gguf";

    /// Creates `files` in a snapshot of `repo_id` under `cache` and returns
    /// the snapshot directory.
    fn cache_repo(cache: &Path, repo_id: &str, files: &[&str]) -> PathBuf {
        let snapshot = repo_snapshots_dir(cache, repo_id).join("abc123");
        std::fs::create_dir_all(&snapshot).unwrap();
        for file in files {
            std::fs::write(snapshot.join(file), b"GGUF").unwrap();
        }
        snapshot
    }

    async fn not_found() -> Result<PathBuf, String> {
        Err("status code 404".to_string())
    }

    async fn must_not_download() -> Result<PathBuf, String> {
        panic!("a cached model must not reach the download step")
    }

    /// 0.98.0 answered a request for Gemma 4 12B with a cached Qwen 27B, and
    /// ran it under the Gemma name until the GPU ran out of memory.
    #[spec("ROUTER-006")]
    #[tokio::test]
    async fn a_failed_download_never_substitutes_a_model_from_another_repo() {
        let cache = tempfile::tempdir().unwrap();
        cache_repo(
            cache.path(),
            "unsloth/Qwen3.5-27B-GGUF",
            &["Qwen3.5-27B-UD-Q6_K_XL.gguf"],
        );

        let err = resolve_gguf_model(Some(cache.path()), GEMMA_REPO, GEMMA_FILE, not_found)
            .await
            .expect_err("another repo's model must not stand in for the requested one");

        assert!(err.contains("status code 404"), "reason missing: {err}");
        assert!(
            err.contains(
                "Download with: hf download ggml-org/gemma-4-12B-it-GGUF gemma-4-12B-it-Q4_0.gguf"
            ),
            "download command missing: {err}"
        );
    }

    #[tokio::test]
    async fn the_cached_file_is_returned_without_a_download() {
        let cache = tempfile::tempdir().unwrap();
        let snapshot = cache_repo(
            cache.path(),
            GEMMA_REPO,
            &["gemma-4-12B-it-BF16.gguf", GEMMA_FILE],
        );

        let found = resolve_gguf_model(
            Some(cache.path()),
            GEMMA_REPO,
            GEMMA_FILE,
            must_not_download,
        )
        .await
        .unwrap();

        assert_eq!(found, snapshot.join(GEMMA_FILE));
    }

    #[tokio::test]
    async fn a_missing_file_is_downloaded() {
        let cache = tempfile::tempdir().unwrap();
        let downloaded = cache.path().join(GEMMA_FILE);
        std::fs::write(&downloaded, b"GGUF").unwrap();

        let found = resolve_gguf_model(Some(cache.path()), GEMMA_REPO, GEMMA_FILE, || async {
            Ok(downloaded.clone())
        })
        .await
        .unwrap();

        assert_eq!(found, downloaded);
    }

    /// Another quantization of the requested model is still that model, so an
    /// offline device keeps working with the build it already has.
    #[tokio::test]
    async fn another_build_from_the_same_repo_is_used_when_the_download_fails() {
        let cache = tempfile::tempdir().unwrap();
        cache_repo(cache.path(), "unsloth/Qwen3.5-27B-GGUF", &["a.gguf"]);
        let snapshot = cache_repo(
            cache.path(),
            GEMMA_REPO,
            &[
                "dflash-gemma-4-12B-it-Q8_0.gguf",
                "gemma-4-12B-it-Q8_0.gguf",
                "mmproj-gemma-4-12B-it-Q8_0.gguf",
                "mtp-gemma-4-12B-it-Q8_0.gguf",
            ],
        );

        let found = resolve_gguf_model(Some(cache.path()), GEMMA_REPO, GEMMA_FILE, not_found)
            .await
            .unwrap();

        assert_eq!(found, snapshot.join("gemma-4-12B-it-Q8_0.gguf"));
    }

    /// A repo cached with only its projector and drafter files holds no model.
    #[spec("ROUTER-006")]
    #[tokio::test]
    async fn companion_files_are_never_returned_as_the_model() {
        let cache = tempfile::tempdir().unwrap();
        cache_repo(
            cache.path(),
            GEMMA_REPO,
            &[
                "dflash-gemma-4-12B-it-Q8_0.gguf",
                "mmproj-gemma-4-12B-it-Q8_0.gguf",
                "MMPROJ-gemma-4-12B-it-BF16.gguf",
                "mtp-gemma-4-12B-it-Q4_0.gguf",
                "mtp-gemma-4-12B-it-Q4_0.gguf.tdf",
            ],
        );

        let err = resolve_gguf_model(Some(cache.path()), GEMMA_REPO, GEMMA_FILE, not_found)
            .await
            .expect_err("a companion file is not a model");

        assert!(err.contains("Download with: hf download"), "{err}");
    }

    #[tokio::test]
    async fn a_device_without_a_cache_directory_reports_the_download_command() {
        let err = resolve_gguf_model(None, GEMMA_REPO, GEMMA_FILE, not_found)
            .await
            .unwrap_err();

        assert!(err.contains(&download_command(GEMMA_REPO, GEMMA_FILE)));
    }

    /// The classifier and judge take any model on disk, but a companion file
    /// is not one: `dflash-` sorts ahead of the weights it ships beside.
    #[test]
    fn the_any_model_scan_skips_companion_files() {
        let cache = tempfile::tempdir().unwrap();
        let snapshot = cache_repo(
            cache.path(),
            "ggml-org/gemma-4-26B-A4B-it-GGUF",
            &[
                "dflash-gemma-4-26B-A4B-it-Q8_0.gguf",
                "gemma-4-26B-A4B-it-Q4_0.gguf",
            ],
        );

        assert_eq!(
            find_small_plain_gguf_in(cache.path(), SMALL_MODEL_SIZE_CEILING),
            Ok(snapshot.join("gemma-4-26B-A4B-it-Q4_0.gguf"))
        );
    }

    #[test]
    fn the_any_model_scan_finds_nothing_in_a_cache_of_companion_files() {
        let cache = tempfile::tempdir().unwrap();
        cache_repo(
            cache.path(),
            GEMMA_REPO,
            &[
                "mmproj-gemma-4-12B-it-Q8_0.gguf",
                "mtp-gemma-4-12B-it-Q4_0.gguf",
            ],
        );

        assert_eq!(
            find_small_plain_gguf_in(cache.path(), SMALL_MODEL_SIZE_CEILING),
            Err(NoSmallModel::NoneCached)
        );
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod small_model_tests {
    use super::*;

    const E2B_REPO: &str = "unsloth/gemma-4-E2B-it-GGUF";
    const E2B_FILE: &str = "gemma-4-E2B-it-Q4_K_M.gguf";
    const ROLE_REPO: &str = "ggml-org/gemma-4-12B-it-GGUF";
    const ROLE_FILE: &str = "gemma-4-12B-it-Q4_0.gguf";

    /// Sizes are scaled down a million to one, so a 7.2 GB model is a
    /// 7,200-byte file and the 5 GB ceiling is 5,000 bytes.
    const CEILING: u64 = 5_000;

    /// Writes a model of `size` bytes into a snapshot of `repo_id`.
    fn cache_model(cache: &Path, repo_id: &str, file: &str, size: usize) -> PathBuf {
        let snapshot = repo_snapshots_dir(cache, repo_id).join("abc123");
        std::fs::create_dir_all(&snapshot).unwrap();
        let path = snapshot.join(file);
        std::fs::write(&path, vec![b'G'; size]).unwrap();
        path
    }

    /// Regression: on a default install (Gemma 4 E2B and 12B) the scan took
    /// the alphabetically first repo, `models--ggml-org--gemma-4-12B…`, so
    /// every agent loaded the 12B role model to classify a task.
    #[test]
    fn a_default_install_classifies_with_the_edge_model_not_the_role_model() {
        let cache = tempfile::tempdir().unwrap();
        cache_model(cache.path(), ROLE_REPO, ROLE_FILE, 7_200);
        let edge = cache_model(cache.path(), E2B_REPO, E2B_FILE, 3_100);

        assert_eq!(find_small_plain_gguf_in(cache.path(), CEILING), Ok(edge));
    }

    #[test]
    fn a_cache_of_role_models_yields_no_classifier_model() {
        let cache = tempfile::tempdir().unwrap();
        let role = cache_model(cache.path(), ROLE_REPO, ROLE_FILE, 7_200);
        cache_model(
            cache.path(),
            "unsloth/Qwen3.8-27B-GGUF",
            "Qwen3.8-27B-Q4_K_M.gguf",
            17_100,
        );

        assert_eq!(
            find_small_plain_gguf_in(cache.path(), CEILING),
            Err(NoSmallModel::AllTooLarge {
                smallest: role,
                size_bytes: 7_200,
            })
        );
    }

    #[test]
    fn the_ceiling_itself_is_too_large() {
        let cache = tempfile::tempdir().unwrap();
        cache_model(cache.path(), "org/exact", "exact.gguf", 5_000);

        assert!(matches!(
            find_small_plain_gguf_in(cache.path(), CEILING),
            Err(NoSmallModel::AllTooLarge {
                size_bytes: 5_000,
                ..
            })
        ));
    }

    #[test]
    fn the_smallest_of_several_unfamiliar_models_is_used() {
        let cache = tempfile::tempdir().unwrap();
        cache_model(cache.path(), "org/a-large", "a-large.gguf", 4_000);
        let small = cache_model(cache.path(), "org/z-small", "z-small.gguf", 900);
        cache_model(cache.path(), "org/m-medium", "m-medium.gguf", 2_000);

        assert_eq!(find_small_plain_gguf_in(cache.path(), CEILING), Ok(small));
    }

    #[test]
    fn the_smallest_build_in_a_repo_is_used() {
        let cache = tempfile::tempdir().unwrap();
        cache_model(cache.path(), E2B_REPO, "gemma-4-E2B-it-BF16.gguf", 4_900);
        let q4 = cache_model(cache.path(), E2B_REPO, E2B_FILE, 3_100);

        assert_eq!(find_small_plain_gguf_in(cache.path(), CEILING), Ok(q4));
    }

    #[test]
    fn a_known_small_chat_model_beats_a_smaller_unfamiliar_one() {
        let cache = tempfile::tempdir().unwrap();
        cache_model(cache.path(), "org/tiny", "tiny.gguf", 100);
        let qwen = cache_model(
            cache.path(),
            "unsloth/Qwen3.5-0.8B-GGUF",
            "Qwen3.5-0.8B-Q4_K_M.gguf",
            530,
        );

        assert_eq!(find_small_plain_gguf_in(cache.path(), CEILING), Ok(qwen));
    }

    #[test]
    fn an_oversized_build_of_a_preferred_model_is_not_used() {
        let cache = tempfile::tempdir().unwrap();
        cache_model(
            cache.path(),
            "mistralai/Ministral-3-3B-Instruct-2512-GGUF",
            "Ministral-3-3B-Instruct-2512-BF16.gguf",
            6_900,
        );
        let edge = cache_model(cache.path(), E2B_REPO, E2B_FILE, 3_100);

        assert_eq!(find_small_plain_gguf_in(cache.path(), CEILING), Ok(edge));
    }

    /// An embedder is often the smallest GGUF in a cache and cannot answer a
    /// classification prompt.
    #[test]
    fn embedders_and_rerankers_are_not_classifier_models() {
        let cache = tempfile::tempdir().unwrap();
        cache_model(
            cache.path(),
            "Qwen/Qwen3-Embedding-0.6B-GGUF",
            "Qwen3-Embedding-0.6B-Q8_0.gguf",
            640,
        );
        cache_model(
            cache.path(),
            "org/reranker",
            "bge-reranker-v2-m3-Q4_K_M.gguf",
            400,
        );
        let edge = cache_model(cache.path(), E2B_REPO, E2B_FILE, 3_100);

        assert_eq!(find_small_plain_gguf_in(cache.path(), CEILING), Ok(edge));
    }

    #[test]
    fn companion_files_are_not_classifier_models() {
        let cache = tempfile::tempdir().unwrap();
        cache_model(
            cache.path(),
            ROLE_REPO,
            "mmproj-gemma-4-12B-it-Q8_0.gguf",
            600,
        );
        cache_model(cache.path(), ROLE_REPO, "mtp-gemma-4-12B-it-Q4_0.gguf", 300);
        cache_model(
            cache.path(),
            ROLE_REPO,
            "dflash-gemma-4-12B-it-Q8_0.gguf",
            200,
        );
        let role = cache_model(cache.path(), ROLE_REPO, ROLE_FILE, 7_200);

        assert_eq!(
            find_small_plain_gguf_in(cache.path(), CEILING),
            Err(NoSmallModel::AllTooLarge {
                smallest: role,
                size_bytes: 7_200,
            })
        );
    }

    #[test]
    fn an_empty_or_missing_cache_yields_nothing() {
        let cache = tempfile::tempdir().unwrap();
        assert_eq!(
            find_small_plain_gguf_in(cache.path(), CEILING),
            Err(NoSmallModel::NoneCached)
        );
        assert_eq!(
            find_small_plain_gguf_in(&cache.path().join("absent"), CEILING),
            Err(NoSmallModel::NoneCached)
        );
    }

    #[test]
    fn the_reason_names_the_model_and_the_ceiling() {
        let reason = NoSmallModel::AllTooLarge {
            smallest: PathBuf::from("/cache/snapshots/x/gemma-4-12B-it-Q4_0.gguf"),
            size_bytes: 7_219_673_216,
        }
        .to_string();

        assert!(reason.contains("gemma-4-12B-it-Q4_0.gguf"), "{reason}");
        assert!(reason.contains("7.2 GB"), "{reason}");
        assert!(reason.contains("5 GB"), "{reason}");
        assert!(!reason.contains("/cache/"), "{reason}");
    }
}
