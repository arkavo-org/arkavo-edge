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

/// Prefers a protected `.gguf.tdf` over a sibling plaintext `.gguf`.
///
/// Wrapping a model is additive, so `model.gguf` and `model.gguf.tdf` often
/// sit side by side. Discovery must not hand back the plaintext and quietly
/// bypass KAS; a TDF-capable loader has to see the protected artifact.
pub fn prefer_protected_model(path: &Path) -> PathBuf {
    let name = path.to_string_lossy();
    if name.to_lowercase().ends_with(PROTECTED_EXTENSION) {
        return path.to_path_buf();
    }
    let protected = PathBuf::from(format!("{name}{}", ".tdf"));
    if name.to_lowercase().ends_with(".gguf") && protected.exists() {
        tracing::info!(
            "preferring protected model {:?} over the sibling plaintext",
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
            return Ok(prefer_protected_model(&path));
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
            return Ok(prefer_protected_model(&path));
        }
        Err(e) => {
            tracing::debug!("find_gguf_model: hf_hub API failed: {}", e);
        }
    }

    // 3. Scan cache for any GGUF in the preferred repo
    if let Some(path) = scan_cache_for_gguf(&api, repo_id).await {
        tracing::debug!("find_gguf_model: found via cache scan at {:?}", path);
        return Ok(path);
    }

    // 4. Fallback: use ANY available .gguf file from cache
    if let Some(path) = find_any_gguf().await {
        tracing::info!(
            "Using fallback model: {}",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
        );
        return Ok(prefer_protected_model(&path));
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

/// Recursively find the first .gguf file in a directory
fn find_gguf_in_dir(dir: &std::path::Path) -> Option<PathBuf> {
    tracing::debug!("find_gguf_in_dir: scanning {:?}", dir);
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_file = path.is_file();
            let ext = path.extension().and_then(|s| s.to_str());
            tracing::debug!(
                "find_gguf_in_dir: entry {:?} is_file={} ext={:?}",
                path.file_name(),
                is_file,
                ext
            );
            if is_file && ext == Some("gguf") {
                tracing::debug!("find_gguf_in_dir: found GGUF at {:?}", path);
                return Some(path);
            } else if path.is_dir()
                && let Some(found) = find_gguf_in_dir(&path)
            {
                return Some(found);
            }
        }
    } else {
        tracing::debug!("find_gguf_in_dir: failed to read dir {:?}", dir);
    }
    tracing::debug!("find_gguf_in_dir: no GGUF found in {:?}", dir);
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
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.file_name().and_then(|s| s.to_str()) == Some(filename) {
                return Some(path);
            } else if path.is_dir()
                && let Some(found) = find_file_in_dir(&path, filename)
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

/// Scan entire HuggingFace cache for any .gguf file
///
/// This is the ultimate fallback when no specific model is found.
/// Prefers TØRG-compatible models (Qwen3, Ministral) over others.
pub async fn find_any_gguf() -> Option<PathBuf> {
    let cache = get_hf_cache_dir()?;

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

    // Check preferred repos first
    for repo_name in &preferred_repos {
        let repo_path = cache.join(repo_name.as_str());
        if repo_path.exists()
            && let Some(gguf) = find_gguf_in_dir(&repo_path)
        {
            return Some(gguf);
        }
    }

    // Fallback: scan all model repositories
    if let Ok(entries) = std::fs::read_dir(&cache) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && path.file_name()?.to_str()?.starts_with("models--")
                && let Some(gguf) = find_gguf_in_dir(&path)
            {
                return Some(gguf);
            }
        }
    }

    None
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
                assert_eq!(path.extension().unwrap(), "gguf");
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

    /// T12/T14: when both artifacts exist, discovery must select the
    /// protected one. Handing back the plaintext would silently bypass KAS.
    #[test]
    fn prefers_the_protected_artifact_when_both_exist() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("model.gguf");
        let protected = dir.path().join("model.gguf.tdf");
        std::fs::write(&plain, b"GGUF").unwrap();
        std::fs::write(&protected, b"PK\x03\x04").unwrap();

        assert_eq!(prefer_protected_model(&plain), protected);
    }

    #[test]
    fn keeps_the_plaintext_when_no_protected_sibling_exists() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("model.gguf");
        std::fs::write(&plain, b"GGUF").unwrap();

        assert_eq!(prefer_protected_model(&plain), plain);
    }

    #[test]
    fn a_protected_path_is_returned_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let protected = dir.path().join("model.gguf.tdf");
        std::fs::write(&protected, b"PK\x03\x04").unwrap();

        assert_eq!(prefer_protected_model(&protected), protected);
        // And it must not look for `model.gguf.tdf.tdf`.
        assert!(!dir.path().join("model.gguf.tdf.tdf").exists());
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("Model.GGUF");
        let protected = dir.path().join("Model.GGUF.tdf");
        std::fs::write(&plain, b"GGUF").unwrap();
        std::fs::write(&protected, b"PK\x03\x04").unwrap();

        assert_eq!(prefer_protected_model(&plain), protected);
    }

    #[test]
    fn a_non_gguf_path_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let other = dir.path().join("notes.txt");
        std::fs::write(&other, b"hello").unwrap();

        assert_eq!(prefer_protected_model(&other), other);
    }
}
