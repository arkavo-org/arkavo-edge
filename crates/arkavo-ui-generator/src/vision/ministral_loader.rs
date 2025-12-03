use anyhow::{Context, Result};
use hf_hub::api::tokio::ApiBuilder;
use std::path::PathBuf;

/// Device-aware quantization selection for edge deployment
#[derive(Debug, Clone, Copy, Default)]
pub enum DeviceProfile {
    /// Raspberry Pi 5 or similar low-memory ARM devices (8GB)
    /// Uses IQ4_XS or Q4_0 - smaller, faster, avoids OOM
    RaspberryPi5,
    /// Desktop or server with dedicated GPU
    /// Uses Q4_K_M or Q5_K_M - better quality
    #[default]
    Desktop,
}

/// Ministral 3 model sizes available from official Mistral repos
#[derive(Debug, Clone, Copy)]
pub enum Ministral3Size {
    /// 3B parameter model - edge-optimized, runs on RPi5
    Small3B,
    /// 8B parameter model - balanced performance
    Medium8B,
    /// 14B parameter model - highest capability
    Large14B,
}

impl Ministral3Size {
    pub fn name(&self) -> &'static str {
        match self {
            Ministral3Size::Small3B => "3B",
            Ministral3Size::Medium8B => "8B",
            Ministral3Size::Large14B => "14B",
        }
    }

    fn repo_id(&self) -> &'static str {
        match self {
            Ministral3Size::Small3B => "mistralai/Ministral-3-3B-Instruct-2512-GGUF",
            Ministral3Size::Medium8B => "mistralai/Ministral-3-8B-Instruct-2512-GGUF",
            Ministral3Size::Large14B => "mistralai/Ministral-3-14B-Instruct-2512-GGUF",
        }
    }
}

/// Paths to downloaded model files
#[derive(Debug, Clone)]
pub struct Ministral3ModelPaths {
    /// Path to the main GGUF model file
    pub model: PathBuf,
    /// Path to the vision projection file (optional - graceful degradation if missing)
    pub mmproj: Option<PathBuf>,
}

pub struct Ministral3ModelLoader {
    cache_dir: PathBuf,
    device_profile: DeviceProfile,
}

impl Ministral3ModelLoader {
    pub fn new(device_profile: DeviceProfile) -> Result<Self> {
        let cache_dir = dirs::home_dir()
            .context("Failed to get home directory")?
            .join(".arkavo")
            .join("models")
            .join("ministral3");

        std::fs::create_dir_all(&cache_dir).context("Failed to create model cache directory")?;

        Ok(Self {
            cache_dir,
            device_profile,
        })
    }

    /// Create loader with automatic device detection
    pub fn new_auto() -> Result<Self> {
        let profile = Self::detect_device_profile();
        Self::new(profile)
    }

    /// Detect device profile based on system resources
    fn detect_device_profile() -> DeviceProfile {
        // Check for Raspberry Pi environment variable
        if std::env::var("ARKAVO_RASPBERRY_PI")
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or(false)
        {
            return DeviceProfile::RaspberryPi5;
        }

        // Check CPU core count as a heuristic
        let num_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);

        if num_cores <= 4 {
            DeviceProfile::RaspberryPi5
        } else {
            DeviceProfile::Desktop
        }
    }

    /// Get the quantization variant based on device profile and model size
    fn get_quantization(&self, size: Ministral3Size) -> &'static str {
        match (&self.device_profile, size) {
            // For RPi5: use smaller quantizations to fit in 8GB RAM
            (DeviceProfile::RaspberryPi5, Ministral3Size::Small3B) => "Q4_K_M",
            (DeviceProfile::RaspberryPi5, Ministral3Size::Medium8B) => "Q4_K_M",
            (DeviceProfile::RaspberryPi5, Ministral3Size::Large14B) => "Q4_K_M",
            // For Desktop: use higher quality quantizations
            (DeviceProfile::Desktop, Ministral3Size::Small3B) => "Q5_K_M",
            (DeviceProfile::Desktop, Ministral3Size::Medium8B) => "Q5_K_M",
            (DeviceProfile::Desktop, Ministral3Size::Large14B) => "Q4_K_M",
        }
    }

    /// Get model filename based on size and quantization
    fn get_model_filename(&self, size: Ministral3Size) -> String {
        let quant = self.get_quantization(size);
        let size_str = match size {
            Ministral3Size::Small3B => "3B",
            Ministral3Size::Medium8B => "8B",
            Ministral3Size::Large14B => "14B",
        };
        format!("Ministral-3-{size_str}-Instruct-2512-{quant}.gguf")
    }

    /// Get mmproj filename (always BF16 for best vision quality)
    fn get_mmproj_filename(size: Ministral3Size) -> String {
        let size_str = match size {
            Ministral3Size::Small3B => "3B",
            Ministral3Size::Medium8B => "8B",
            Ministral3Size::Large14B => "14B",
        };
        format!("Ministral-3-{size_str}-Instruct-2512-BF16-mmproj.gguf")
    }

    pub async fn load_model(&self, size: Ministral3Size) -> Result<Ministral3ModelPaths> {
        let api = ApiBuilder::new()
            .with_progress(true)
            .build()
            .context("Failed to create HuggingFace API client")?;

        let repo_id = size.repo_id();
        let model_file = self.get_model_filename(size);
        let mmproj_file = Self::get_mmproj_filename(size);

        println!(
            "Downloading Ministral 3 {} model from HuggingFace...",
            size.name()
        );
        println!("Repository: {}", repo_id);
        println!("Device profile: {:?}", self.device_profile);

        let repo = api.model(repo_id.to_string());

        // Download main model file
        println!("Downloading model file: {}", model_file);
        let model_path = repo
            .get(&model_file)
            .await
            .with_context(|| format!("Failed to download model file: {}", model_file))?;

        // Attempt to download vision projector, but don't fail if unavailable
        let mmproj_path = match repo.get(&mmproj_file).await {
            Ok(path) => {
                println!("✓ Vision projector downloaded: {}", mmproj_file);
                Some(path)
            }
            Err(e) => {
                eprintln!(
                    "⚠ Vision capabilities unavailable for Ministral {}: {}",
                    size.name(),
                    e
                );
                println!("⚠ Vision projector not available, text-only mode enabled");
                None
            }
        };

        println!("✓ Model downloaded successfully!");
        println!("  Model: {}", model_path.display());
        if let Some(ref mmproj) = mmproj_path {
            println!("  MMProj: {}", mmproj.display());
        }

        Ok(Ministral3ModelPaths {
            model: model_path,
            mmproj: mmproj_path,
        })
    }

    pub fn get_cached_paths(&self, size: Ministral3Size) -> Option<Ministral3ModelPaths> {
        let model_file = self.get_model_filename(size);
        let mmproj_file = Self::get_mmproj_filename(size);

        let model_path = self.cache_dir.join(&model_file);

        if !model_path.exists() {
            return None;
        }

        let mmproj_path = self.cache_dir.join(&mmproj_file);
        let mmproj = if mmproj_path.exists() {
            Some(mmproj_path)
        } else {
            None
        };

        Some(Ministral3ModelPaths {
            model: model_path,
            mmproj,
        })
    }

    pub async fn ensure_model(&self, size: Ministral3Size) -> Result<Ministral3ModelPaths> {
        if let Some(paths) = self.get_cached_paths(size) {
            println!("✓ Using cached Ministral {} model", size.name());
            println!("  Model: {}", paths.model.display());
            if let Some(ref mmproj) = paths.mmproj {
                println!("  MMProj: {}", mmproj.display());
            }
            Ok(paths)
        } else {
            println!("Model not found in cache, downloading...");
            self.load_model(size).await
        }
    }

    /// Get recommended model size based on device profile
    pub fn recommended_size(&self) -> Ministral3Size {
        match self.device_profile {
            DeviceProfile::RaspberryPi5 => Ministral3Size::Small3B,
            DeviceProfile::Desktop => Ministral3Size::Medium8B,
        }
    }
}

impl Default for Ministral3ModelLoader {
    fn default() -> Self {
        Self::new_auto().expect("Failed to create model loader")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loader_creation() {
        let loader = Ministral3ModelLoader::new(DeviceProfile::Desktop);
        assert!(loader.is_ok());
    }

    #[test]
    fn test_cache_dir_created() {
        let loader = Ministral3ModelLoader::new(DeviceProfile::Desktop).unwrap();
        assert!(loader.cache_dir.exists());
    }

    #[test]
    fn test_model_filename_generation() {
        let desktop_loader = Ministral3ModelLoader::new(DeviceProfile::Desktop).unwrap();
        assert_eq!(
            desktop_loader.get_model_filename(Ministral3Size::Small3B),
            "Ministral-3-3B-Instruct-2512-Q5_K_M.gguf"
        );

        let pi_loader = Ministral3ModelLoader::new(DeviceProfile::RaspberryPi5).unwrap();
        assert_eq!(
            pi_loader.get_model_filename(Ministral3Size::Small3B),
            "Ministral-3-3B-Instruct-2512-Q4_K_M.gguf"
        );
    }

    #[test]
    fn test_mmproj_filename_generation() {
        assert_eq!(
            Ministral3ModelLoader::get_mmproj_filename(Ministral3Size::Small3B),
            "Ministral-3-3B-Instruct-2512-BF16-mmproj.gguf"
        );
        assert_eq!(
            Ministral3ModelLoader::get_mmproj_filename(Ministral3Size::Medium8B),
            "Ministral-3-8B-Instruct-2512-BF16-mmproj.gguf"
        );
        assert_eq!(
            Ministral3ModelLoader::get_mmproj_filename(Ministral3Size::Large14B),
            "Ministral-3-14B-Instruct-2512-BF16-mmproj.gguf"
        );
    }

    #[test]
    fn test_device_profile_quantization() {
        let desktop = Ministral3ModelLoader::new(DeviceProfile::Desktop).unwrap();
        assert_eq!(desktop.get_quantization(Ministral3Size::Small3B), "Q5_K_M");
        assert_eq!(desktop.get_quantization(Ministral3Size::Large14B), "Q4_K_M");

        let pi = Ministral3ModelLoader::new(DeviceProfile::RaspberryPi5).unwrap();
        assert_eq!(pi.get_quantization(Ministral3Size::Small3B), "Q4_K_M");
    }

    #[test]
    fn test_recommended_size() {
        let desktop = Ministral3ModelLoader::new(DeviceProfile::Desktop).unwrap();
        assert!(matches!(
            desktop.recommended_size(),
            Ministral3Size::Medium8B
        ));

        let pi = Ministral3ModelLoader::new(DeviceProfile::RaspberryPi5).unwrap();
        assert!(matches!(pi.recommended_size(), Ministral3Size::Small3B));
    }
}
