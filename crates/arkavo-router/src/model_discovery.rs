/// Model discovery utilities using hf-hub API
///
/// Future: This will be used by `arkavo models list` command and
/// `.arkavo/AGENTS.md` configuration for remote model subsets.
use std::path::PathBuf;

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
    use hf_hub::api::tokio::Api;

    tracing::debug!(
        "find_gguf_model: looking for repo={} filename={}",
        repo_id,
        filename
    );

    let api = Api::new().map_err(|e| format!("Failed to initialize HuggingFace API: {e}"))?;

    // 1. Try preferred model first (download if needed, or use cached)
    let repo = api.repo(hf_hub::Repo::model(repo_id.to_string()));
    match repo.get(filename).await {
        Ok(path) => {
            tracing::debug!("find_gguf_model: found via hf_hub API at {:?}", path);
            return Ok(path);
        }
        Err(e) => {
            tracing::debug!("find_gguf_model: hf_hub API failed: {}", e);
        }
    }

    // 2. Scan cache for any GGUF in the preferred repo
    if let Some(path) = scan_cache_for_gguf(&api, repo_id).await {
        tracing::debug!("find_gguf_model: found via cache scan at {:?}", path);
        return Ok(path);
    }

    // 3. Fallback: use ANY available .gguf file from cache
    if let Some(path) = find_any_gguf().await {
        tracing::info!(
            "Using fallback model: {}",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
        );
        return Ok(path);
    }

    // 4. Nothing found - provide helpful error
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
/// Used by upgrade_model to only upgrade to models that are available.
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

/// Scan entire HuggingFace cache for any .gguf file
///
/// This is the ultimate fallback when no specific model is found.
/// Prefers TØRG-compatible models (Qwen3, Ministral) over others.
pub async fn find_any_gguf() -> Option<PathBuf> {
    let cache = get_hf_cache_dir()?;

    // Priority order: prefer TØRG-compatible models (Qwen3, Ministral)
    let preferred_repos = [
        "models--Qwen--Qwen3-0.6B-GGUF",
        "models--mistralai--Ministral-3-3B-Instruct-2512-GGUF",
        "models--mistralai--Ministral-3-8B-Instruct-2512-GGUF",
    ];

    // Check preferred repos first
    for repo_name in &preferred_repos {
        let repo_path = cache.join(repo_name);
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

    #[tokio::test]
    async fn test_find_gguf_model() {
        // This test will only pass if the model is already cached
        // or if network is available
        let result = find_gguf_model("Qwen/Qwen3-0.6B-GGUF", "Qwen3-0.6B-Q8_0.gguf").await;

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
