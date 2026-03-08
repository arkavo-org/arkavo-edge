//! First-run experience for new Arkavo users
//!
//! Handles system capability detection, model recommendations, and guided setup.

use std::io::{self, Write};
use std::path::PathBuf;

/// Device profile based on system capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceProfile {
    /// Raspberry Pi 5 or similar (<16GB RAM)
    RaspberryPi5,
    /// Desktop/laptop (16-31GB RAM)
    Desktop,
    /// Workstation (32-63GB RAM) - GLM-4.7-Flash capable with capped context
    Workstation,
    /// High-memory workstation (64GB+ RAM) - GLM-4.7-Flash with full 131k context
    HighMemoryWorkstation,
}

impl std::fmt::Display for DeviceProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceProfile::RaspberryPi5 => write!(f, "Embedded (RPi5-class)"),
            DeviceProfile::Desktop => write!(f, "Desktop"),
            DeviceProfile::Workstation => write!(f, "Workstation"),
            DeviceProfile::HighMemoryWorkstation => write!(f, "High-Memory Workstation"),
        }
    }
}

/// Recommended model based on device capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendedModel {
    /// Qwen3.5 0.8B - smallest, for RPi5 (~550MB)
    Qwen35_0_8B,
    /// Ministral 3B - medium, for desktops (~2GB)
    Ministral3B,
    /// Ministral 8B - larger, for desktops (~5GB)
    Ministral8B,
    /// Qwen3.5-27B - 27B dense, requires 48GB+ RAM (~23GB)
    Qwen35_27B,
    /// GLM-4.7-Flash - 30B MoE, requires 32GB+ RAM (~20GB)
    Glm47Flash,
}

impl RecommendedModel {
    /// Corresponding ModelChoice for metadata delegation
    fn model_choice(self) -> arkavo_router::decision::ModelChoice {
        use arkavo_router::decision::ModelChoice;
        match self {
            Self::Qwen35_0_8B => ModelChoice::LocalQwen3,
            Self::Ministral3B => ModelChoice::LocalMinistral3B,
            Self::Ministral8B => ModelChoice::LocalMinistral8B,
            Self::Qwen35_27B => ModelChoice::LocalQwen35_27B,
            Self::Glm47Flash => ModelChoice::LocalGlm47Flash,
        }
    }

    /// HuggingFace repository ID — delegates to [`ModelChoice::repo_id`].
    pub fn repo_id(&self) -> &'static str {
        // SAFETY: all RecommendedModel variants map to local ModelChoice variants
        // which always return Some from repo_id().
        self.model_choice().repo_id().unwrap_or("")
    }

    /// GGUF filename to download — delegates to [`ModelChoice::gguf_filename`].
    pub fn filename(&self) -> &'static str {
        self.model_choice().gguf_filename().unwrap_or("")
    }

    /// Approximate size in bytes
    pub fn size_bytes(&self) -> u64 {
        self.model_choice().size_bytes()
    }

    /// Human-readable display name
    pub fn display_name(&self) -> &'static str {
        self.model_choice().display_name()
    }
}

/// System capabilities detected at startup
#[derive(Debug, Clone)]
pub struct SystemCapabilities {
    pub cpu_cores: usize,
    pub total_ram_gb: u64,
    pub has_unified_memory: bool,
    pub available_disk_gb: f64,
    pub device_profile: DeviceProfile,
    pub recommended_model: RecommendedModel,
}

/// Check if this is a first run (no GGUF models in cache)
pub fn is_first_run() -> bool {
    // Synchronous check using blocking I/O
    let Some(cache) = get_hf_cache_dir() else {
        return true;
    };

    // Quick check for preferred model directories
    use arkavo_router::decision::ModelChoice;
    let preferred_repos: Vec<String> = [
        ModelChoice::LocalQwen3,
        ModelChoice::LocalMinistral3B,
        ModelChoice::LocalMinistral8B,
        ModelChoice::LocalQwen35_27B,
        ModelChoice::LocalGlm47Flash,
    ]
    .iter()
    .filter_map(ModelChoice::cache_dir_name)
    .collect();

    for repo_name in &preferred_repos {
        let repo_path = cache.join(repo_name.as_str());
        if repo_path.exists() && has_gguf_file(&repo_path) {
            return false;
        }
    }

    // Fallback: check if any models--* directory has a .gguf file
    if let Ok(entries) = std::fs::read_dir(&cache) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|name| name.starts_with("models--"))
                && has_gguf_file(&path)
            {
                return false;
            }
        }
    }

    true
}

/// Check if a directory contains any .gguf file (recursive)
fn has_gguf_file(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_gguf = path.is_file() && path.extension().is_some_and(|ext| ext == "gguf");
        if is_gguf || (path.is_dir() && has_gguf_file(&path)) {
            return true;
        }
    }
    false
}

/// Get HuggingFace cache directory
fn get_hf_cache_dir() -> Option<PathBuf> {
    if let Ok(hf_home) = std::env::var("HF_HOME") {
        return Some(PathBuf::from(hf_home).join("hub"));
    }
    dirs::home_dir().map(|home| home.join(".cache").join("huggingface").join("hub"))
}

pub use crate::hardware::calculate_glm_max_context;
use crate::hardware::{detect_unified_memory, get_total_ram_gb};

/// Detect system capabilities
pub fn detect_capabilities() -> SystemCapabilities {
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    let total_ram_gb = get_total_ram_gb();
    let has_unified_memory = detect_unified_memory();

    // Check for Raspberry Pi environment variable override
    let is_rpi = std::env::var("ARKAVO_RASPBERRY_PI")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false);

    // Profile based on RAM (primary) and CPU cores (secondary)
    let device_profile = if is_rpi || total_ram_gb < 16 {
        DeviceProfile::RaspberryPi5
    } else if total_ram_gb >= 64 && cpu_cores >= 8 {
        DeviceProfile::HighMemoryWorkstation
    } else if total_ram_gb >= 32 && cpu_cores >= 6 {
        DeviceProfile::Workstation
    } else {
        DeviceProfile::Desktop
    };

    let recommended_model = match device_profile {
        DeviceProfile::RaspberryPi5 => RecommendedModel::Qwen35_0_8B,
        DeviceProfile::Desktop => RecommendedModel::Ministral8B,
        DeviceProfile::Workstation => RecommendedModel::Glm47Flash,
        DeviceProfile::HighMemoryWorkstation => RecommendedModel::Qwen35_27B,
    };

    // Warn about MoE performance on CPU-only systems
    if matches!(
        device_profile,
        DeviceProfile::Workstation | DeviceProfile::HighMemoryWorkstation
    ) && !has_unified_memory
    {
        eprintln!("Note: GLM-4.7-Flash is a 30B MoE model. On CPU RAM, expect 1-3 tokens/sec.");
    }

    let available_disk_gb = get_available_disk_space();

    SystemCapabilities {
        cpu_cores,
        total_ram_gb,
        has_unified_memory,
        available_disk_gb,
        device_profile,
        recommended_model,
    }
}

/// Get available disk space in GB for HuggingFace cache directory
#[cfg(unix)]
pub fn get_available_disk_space() -> f64 {
    use std::ffi::CString;
    use std::mem::MaybeUninit;

    let cache_path = get_hf_cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));

    // Find an existing directory to check disk space (walk up until we find one)
    let mut check_path = cache_path;
    while !check_path.exists() {
        match check_path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => check_path = parent.to_path_buf(),
            _ => {
                // Fall back to home directory or root
                check_path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                break;
            }
        }
    }

    let path_cstr = match CString::new(check_path.to_string_lossy().as_bytes()) {
        Ok(s) => s,
        Err(_) => return 0.0,
    };

    // SAFETY: path_cstr is a valid CString; statvfs writes to MaybeUninit; return value checked
    unsafe {
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        if libc::statvfs(path_cstr.as_ptr(), stat.as_mut_ptr()) == 0 {
            let stat = stat.assume_init();
            #[allow(clippy::unnecessary_cast)] // Types differ between macOS (u32) and Linux (u64)
            let available = stat.f_bavail as u64 * stat.f_frsize as u64;
            return available as f64 / 1_000_000_000.0;
        }
    }

    0.0
}

#[cfg(windows)]
pub fn get_available_disk_space() -> f64 {
    // Windows implementation would use GetDiskFreeSpaceExW
    // For now, return a reasonable default
    50.0
}

/// Download a model using hf-hub
pub async fn download_model(model: &RecommendedModel) -> Result<PathBuf, String> {
    use hf_hub::api::tokio::ApiBuilder;

    println!("Downloading {}...", model.display_name());
    println!("This may take a few minutes depending on your connection.");

    let api = ApiBuilder::new()
        .with_progress(true)
        .build()
        .map_err(|e| format!("Failed to initialize HuggingFace API: {e}"))?;

    let repo = api.repo(hf_hub::Repo::model(model.repo_id().to_string()));

    match repo.get(model.filename()).await {
        Ok(path) => {
            println!("Download complete: {}", path.display());
            Ok(path)
        }
        Err(e) => Err(format!("Download failed: {e}")),
    }
}

/// Run a simple test query to verify the model works
pub async fn run_test_query() -> Result<String, String> {
    use arkavo_llm::{LlmClient, Message};

    println!("\nTesting model...");

    let client = LlmClient::from_env().map_err(|e| format!("Failed to create LLM client: {e}"))?;

    let response = client
        .complete(vec![Message::user(
            "Say exactly: Hello! Arkavo Edge is ready.",
        )])
        .await
        .map_err(|e| format!("Test query failed: {e}"))?;

    println!("Model: {response}");

    if response.to_lowercase().contains("ready") {
        println!("Model verified successfully.");
    }

    Ok(response)
}

/// Prompt user for download confirmation
pub fn prompt_download_confirmation(caps: &SystemCapabilities) -> bool {
    let model_size_gb = caps.recommended_model.size_bytes() as f64 / 1_000_000_000.0;

    if caps.available_disk_gb < model_size_gb * 1.5 {
        eprintln!(
            "Warning: Low disk space. Need {:.1} GB, have {:.1} GB available.",
            model_size_gb, caps.available_disk_gb
        );
        return false;
    }

    print!(
        "\nDownload {} ({:.1} GB)? (Y/n) ",
        caps.recommended_model.display_name(),
        model_size_gb
    );
    let _ = io::stdout().flush();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }

    let input = input.trim().to_lowercase();
    input.is_empty() || input == "y" || input == "yes"
}

/// Prompt user for downloading both small and large models
pub fn prompt_download_both(caps: &SystemCapabilities, total_gb: f64) -> bool {
    if caps.available_disk_gb < total_gb * 1.2 {
        eprintln!(
            "Warning: Low disk space. Need {:.1} GB, have {:.1} GB available.",
            total_gb, caps.available_disk_gb
        );
        return false;
    }

    print!("\nDownload both models ({total_gb:.1} GB total)? (Y/n) ");
    let _ = io::stdout().flush();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }

    let input = input.trim().to_lowercase();
    input.is_empty() || input == "y" || input == "yes"
}

// Re-export from welcome module for backwards compatibility
pub use crate::welcome::display_welcome_verbose;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_capabilities() {
        let caps = detect_capabilities();
        assert!(caps.cpu_cores > 0);
        assert!(caps.total_ram_gb > 0);
        assert!(caps.available_disk_gb >= 0.0);
    }

    #[test]
    fn test_model_info() {
        let model = RecommendedModel::Qwen35_0_8B;
        assert!(!model.repo_id().is_empty());
        assert!(!model.filename().is_empty());
        assert!(model.size_bytes() > 0);

        let glm = RecommendedModel::Glm47Flash;
        assert!(glm.repo_id().contains("unsloth"));
        assert!(glm.size_bytes() > 10_000_000_000);
    }

    #[test]
    fn test_device_profile_display() {
        assert_eq!(
            DeviceProfile::HighMemoryWorkstation.to_string(),
            "High-Memory Workstation"
        );
        assert_eq!(DeviceProfile::Workstation.to_string(), "Workstation");
    }
}
