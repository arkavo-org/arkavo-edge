/// Model discovery utilities using hf-hub API
///
/// Future: This will be used by `arkavo models list` command and
/// `.arkavo/AGENTS.md` configuration for remote model subsets.
use std::path::{Path, PathBuf};

/// Load API keys from .arkavo/AGENTS.md if present
///
/// This searches for .arkavo/AGENTS.md in the current directory and parent directories,
/// and sets any API_KEY environment variables found in the file.
pub fn load_api_keys_from_config() {
    // Try to find .arkavo/AGENTS.md in current directory or parent directories
    let mut current_dir = std::env::current_dir().ok();
    let mut agents_file = None;

    while let Some(dir) = current_dir {
        let candidate = dir.join(".arkavo").join("AGENTS.md");
        if candidate.exists() {
            agents_file = Some(candidate);
            break;
        }
        current_dir = dir.parent().map(|p| p.to_path_buf());
    }

    if let Some(path) = agents_file
        && let Ok(content) = std::fs::read_to_string(path)
    {
        for line in content.lines() {
            let trimmed = line.trim();

            // Look for API key assignments (e.g., GEMINI_API_KEY=...)
            if trimmed.contains("API_KEY=")
                && let Some((key, value)) = trimmed.split_once('=')
            {
                let key = key.trim();
                let value = value.trim();
                // Only set if not already set in environment
                if std::env::var(key).is_err() {
                    // SAFETY: We're setting environment variables during initialization
                    // before any threads are spawned, so this is safe
                    unsafe {
                        std::env::set_var(key, value);
                    }
                }
            }
        }
    }
}

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

/// Find a GGUF model file, preferring specific models but accepting any available
///
/// Priority:
/// 1. Try to download/use preferred model if available
/// 2. Scan HF cache for the preferred repo
/// 3. Scan HF cache for any .gguf file from any repo
///
/// # Arguments
/// * `repo_id` - Preferred HuggingFace repository ID (e.g., "unsloth/gemma-3-270m-it-GGUF")
/// * `filename` - Preferred GGUF filename (e.g., "gemma-3-270m-it-Q4_0.gguf")
///
/// # Returns
/// * `Ok(PathBuf)` - Path to the model file
/// * `Err(String)` - Error with user-friendly message including download instructions
pub async fn find_gguf_model(repo_id: &str, filename: &str) -> Result<PathBuf, String> {
    tracing::debug!(
        "find_gguf_model: looking for repo={} filename={}",
        repo_id,
        filename
    );

    // 1. Check local cache first (no network, instant)
    if let Some(cache) = get_hf_cache_dir() {
        let repo_cache_name = format!("models--{}", repo_id.replace('/', "--"));
        let snapshots_dir = cache.join(&repo_cache_name).join("snapshots");
        if let Some(path) = find_file_in_dir(&snapshots_dir, filename) {
            tracing::debug!("find_gguf_model: found in local cache at {:?}", path);
            return Ok(resolve_gguf_path(&path));
        }
    }

    // 2. Not cached — try downloading via hf_hub API
    use hf_hub::api::tokio::Api;
    let api = Api::new().map_err(|e| format!("Failed to initialize HuggingFace API: {e}"))?;

    let repo = api.repo(hf_hub::Repo::model(repo_id.to_string()));
    match repo.get(filename).await {
        Ok(path) => {
            tracing::debug!(
                "find_gguf_model: downloaded/found via hf_hub API at {:?}",
                path
            );
            return Ok(resolve_gguf_path(&path));
        }
        Err(e) => {
            tracing::debug!("find_gguf_model: hf_hub API failed: {}", e);
        }
    }

    // 3. Scan cache for any GGUF in the preferred repo
    if let Some(path) = scan_cache_for_gguf(&api, repo_id).await {
        tracing::debug!("find_gguf_model: found via cache scan at {:?}", path);
        return Ok(resolve_gguf_path(&path));
    }

    // 4. Fallback: use ANY available .gguf file from cache
    if let Some(path) = find_any_gguf().await {
        tracing::info!(
            "Using fallback model: {}",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
        );
        return Ok(resolve_gguf_path(&path));
    }

    // 5. Nothing found - provide helpful error
    Err(format!(
        "No GGUF models found in HuggingFace cache. Download with: hf download {repo_id} {filename}"
    ))
}

/// Scan HuggingFace cache for any .gguf file in a repository
///
/// This is a fallback when the preferred model file isn't found.
/// Looks for any .gguf file in the cache snapshots directory.
async fn scan_cache_for_gguf(_api: &hf_hub::api::tokio::Api, repo_id: &str) -> Option<PathBuf> {
    // Get standard HuggingFace cache location
    let cache = get_hf_cache_dir()?;
    tracing::debug!("scan_cache_for_gguf: cache_dir={:?}", cache);

    // Convert repo_id to cache directory format: "org/model" -> "models--org--model"
    let repo_cache_name = format!("models--{}", repo_id.replace('/', "--"));
    let repo_cache_path = cache.join(&repo_cache_name);
    tracing::debug!(
        "scan_cache_for_gguf: repo_cache_path={:?} exists={}",
        repo_cache_path,
        repo_cache_path.exists()
    );

    if !repo_cache_path.exists() {
        return None;
    }

    // Scan snapshots directory for .gguf files
    let snapshots_dir = repo_cache_path.join("snapshots");
    tracing::debug!(
        "scan_cache_for_gguf: snapshots_dir={:?} exists={}",
        snapshots_dir,
        snapshots_dir.exists()
    );

    if !snapshots_dir.exists() {
        return None;
    }

    // Recursively search for .gguf files
    let result = find_gguf_in_dir(&snapshots_dir);
    tracing::debug!("scan_cache_for_gguf: found={:?}", result);
    result
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
/// artifact in `protected` when no plaintext has been found yet.
fn find_artifact_in_dir(dir: &std::path::Path, protected: &mut Option<PathBuf>) -> Option<PathBuf> {
    tracing::debug!("find_artifact_in_dir: scanning {:?}", dir);
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_file() {
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
    let Some(cache) = get_hf_cache_dir() else {
        return false;
    };

    // Convert repo_id to cache directory format: "org/model" -> "models--org--model"
    let repo_cache_name = format!("models--{}", repo_id.replace('/', "--"));
    let repo_cache_path = cache.join(&repo_cache_name);

    if !repo_cache_path.exists() {
        return false;
    }

    // Check snapshots directory for the specific file
    let snapshots_dir = repo_cache_path.join("snapshots");
    if !snapshots_dir.exists() {
        return false;
    }

    // Search for the exact filename
    find_file_in_dir(&snapshots_dir, filename).is_some()
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

/// Scan the HuggingFace cache for any **plaintext** `.gguf`.
///
/// This is the fallback the routing classifier and response judge use. They
/// construct `LlamaCppProvider` synchronously and cannot rewrap a protected
/// model, so a `.gguf.tdf` is never a candidate here — a cache holding only
/// protected models yields `None` and the caller falls back to rule-based
/// classification instead of failing router init.
pub async fn find_any_gguf() -> Option<PathBuf> {
    let cache = get_hf_cache_dir()?;
    find_any_plain_gguf_in(&cache)
}

fn find_any_plain_gguf_in(cache: &Path) -> Option<PathBuf> {
    // Priority order: prefer smallest models first — classifier/judge need speed, not quality.
    // Loading large models here wastes memory (bypasses per-agent memory budget).
    use crate::decision::ModelChoice;
    let preferred_repos: Vec<String> = [
        ModelChoice::LocalQwen3,
        ModelChoice::LocalMinistral3B,
        ModelChoice::LocalMinistral8B,
        ModelChoice::LocalQwen35_27B,
    ]
    .iter()
    .filter_map(ModelChoice::cache_dir_name)
    .collect();

    for repo_name in &preferred_repos {
        let repo_path = cache.join(repo_name.as_str());
        if repo_path.exists()
            && let Some(gguf) = find_plain_gguf_in_dir(&repo_path)
        {
            return Some(gguf);
        }
    }

    let mut repos: Vec<PathBuf> = std::fs::read_dir(cache)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("models--"))
        })
        .collect();
    repos.sort();
    repos.iter().find_map(|p| find_plain_gguf_in_dir(p))
}

/// Recursively find a plaintext `.gguf`, ignoring `.gguf.tdf`.
fn find_plain_gguf_in_dir(dir: &Path) -> Option<PathBuf> {
    let mut ignored = None;
    find_artifact_in_dir(dir, &mut ignored)
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

    #[spec("ROUTER-006")]
    #[tokio::test]
    async fn test_find_gguf_model() {
        // This test will only pass if the model is already cached
        // or if network is available
        use crate::decision::ModelChoice;
        let repo = ModelChoice::LocalQwen3.repo_id().unwrap();
        let file = ModelChoice::LocalQwen3.gguf_filename().unwrap();
        let result = find_gguf_model(repo, file).await;

        match result {
            Ok(path) => {
                assert!(path.exists());
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                assert!(
                    name.to_lowercase().ends_with(".gguf")
                        || name.to_lowercase().ends_with(PROTECTED_EXTENSION),
                    "unexpected model artifact {name}"
                );
            }
            Err(e) => {
                // Expected if model not cached and no network
                assert!(e.contains("Download with:"));
            }
        }
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

        assert_eq!(find_any_plain_gguf_in(cache.path()), None);
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

        let found = find_any_plain_gguf_in(cache.path()).unwrap();
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

        let found = find_any_plain_gguf_in(cache.path()).unwrap();
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
