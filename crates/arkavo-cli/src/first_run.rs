//! First-run experience for new Arkavo users
//!
//! Handles system capability detection, model recommendations, and guided setup.

use std::io::{self, Write};
use std::path::PathBuf;

/// Device profile based on system capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceProfile {
    /// Raspberry Pi 5 or similar (≤4 cores, 8GB RAM)
    RaspberryPi5,
    /// Desktop/laptop (>4 cores)
    Desktop,
}

impl std::fmt::Display for DeviceProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceProfile::RaspberryPi5 => write!(f, "Embedded (RPi5-class)"),
            DeviceProfile::Desktop => write!(f, "Desktop"),
        }
    }
}

/// Recommended model based on device capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecommendedModel {
    /// Qwen3 0.6B - smallest, for RPi5 (~600MB)
    Qwen3_0_6B,
    /// Ministral 3B - medium, for desktops (~2GB)
    Ministral3B,
    /// Ministral 8B - larger, for workstations (~5GB)
    Ministral8B,
}

impl RecommendedModel {
    /// HuggingFace repository ID
    pub fn repo_id(&self) -> &'static str {
        match self {
            RecommendedModel::Qwen3_0_6B => "Qwen/Qwen3-0.6B-GGUF",
            RecommendedModel::Ministral3B => "mistralai/Ministral-3-3B-Instruct-2512-GGUF",
            RecommendedModel::Ministral8B => "mistralai/Ministral-3-8B-Instruct-2512-GGUF",
        }
    }

    /// GGUF filename to download
    pub fn filename(&self) -> &'static str {
        match self {
            RecommendedModel::Qwen3_0_6B => "Qwen3-0.6B-Q8_0.gguf",
            RecommendedModel::Ministral3B => "Ministral-3-3B-Instruct-2512-Q5_K_M.gguf",
            RecommendedModel::Ministral8B => "Ministral-3-8B-Instruct-2512-Q5_K_M.gguf",
        }
    }

    /// Approximate size in bytes
    pub fn size_bytes(&self) -> u64 {
        match self {
            RecommendedModel::Qwen3_0_6B => 650_000_000,    // ~650MB
            RecommendedModel::Ministral3B => 2_500_000_000, // ~2.5GB
            RecommendedModel::Ministral8B => 5_500_000_000, // ~5.5GB
        }
    }

    /// Human-readable display name
    pub fn display_name(&self) -> &'static str {
        match self {
            RecommendedModel::Qwen3_0_6B => "Qwen3 0.6B",
            RecommendedModel::Ministral3B => "Ministral 3B",
            RecommendedModel::Ministral8B => "Ministral 8B",
        }
    }
}

/// System capabilities detected at startup
#[derive(Debug, Clone)]
pub struct SystemCapabilities {
    pub cpu_cores: usize,
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
    let preferred_repos = [
        "models--Qwen--Qwen3-0.6B-GGUF",
        "models--mistralai--Ministral-3-3B-Instruct-2512-GGUF",
        "models--mistralai--Ministral-3-8B-Instruct-2512-GGUF",
    ];

    for repo_name in &preferred_repos {
        let repo_path = cache.join(repo_name);
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

/// Detect system capabilities
pub fn detect_capabilities() -> SystemCapabilities {
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    // Check for Raspberry Pi environment variable override
    let is_rpi = std::env::var("ARKAVO_RASPBERRY_PI")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false);

    let device_profile = if is_rpi || cpu_cores <= 4 {
        DeviceProfile::RaspberryPi5
    } else {
        DeviceProfile::Desktop
    };

    let recommended_model = match device_profile {
        DeviceProfile::RaspberryPi5 => RecommendedModel::Qwen3_0_6B,
        DeviceProfile::Desktop => RecommendedModel::Ministral3B,
    };

    let available_disk_gb = get_available_disk_space();

    SystemCapabilities {
        cpu_cores,
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

    // Ensure parent directory exists for statvfs
    let check_path = if cache_path.exists() {
        cache_path
    } else {
        cache_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/"))
    };

    let path_cstr = match CString::new(check_path.to_string_lossy().as_bytes()) {
        Ok(s) => s,
        Err(_) => return 0.0,
    };

    unsafe {
        let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
        if libc::statvfs(path_cstr.as_ptr(), stat.as_mut_ptr()) == 0 {
            let stat = stat.assume_init();
            let available = stat.f_bavail as u64 * stat.f_frsize;
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
    use arkavo_llm::{LlmClient, Message, Role};

    println!("\nTesting model...");

    let client = LlmClient::from_env().map_err(|e| format!("Failed to create LLM client: {e}"))?;

    let response = client
        .complete(vec![Message {
            role: Role::User,
            content: "Say exactly: Hello! Arkavo Edge is ready.".to_string(),
            images: None,
        }])
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

/// Display welcome message with QR code (verbose mode)
pub fn display_welcome_verbose() -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_crypto::AgentKeypair;
    use arkavo_device_identity::{get_or_create_device_id, keypair};
    use arkavo_registration::{AgentDescriptor, qr::display_qr};

    println!("Welcome Friend\n");

    // Get or create device ID
    let _device_id = get_or_create_device_id()?;

    // Get or create keypair
    let keypair_bytes = match keypair::get_keypair()? {
        Some(bytes) => bytes,
        None => {
            let new_keypair = AgentKeypair::generate();
            let bytes = new_keypair.to_bytes();
            keypair::store_keypair(&bytes)?;
            bytes
        }
    };

    let agent_keypair = AgentKeypair::from_bytes(&keypair_bytes)?;
    let public_key = agent_keypair.public_key();

    // Get hostname for endpoint
    let hostname = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "localhost".to_string());

    // Create agent descriptor with public key
    let short_id = &public_key.to_base64()[..7.min(public_key.to_base64().len())];
    let descriptor = AgentDescriptor::new(
        public_key,
        format!("http://{hostname}:8342"),
        Some(format!("{hostname}._a2a._tcp.local.")),
        short_id.to_string(),
    );

    // Display QR code
    display_qr(&descriptor)?;

    println!();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_capabilities() {
        let caps = detect_capabilities();
        assert!(caps.cpu_cores > 0);
        assert!(caps.available_disk_gb >= 0.0);
    }

    #[test]
    fn test_model_info() {
        let model = RecommendedModel::Qwen3_0_6B;
        assert!(!model.repo_id().is_empty());
        assert!(!model.filename().is_empty());
        assert!(model.size_bytes() > 0);
    }
}
