use anyhow::{Context, Result};
use hf_hub::api::tokio::ApiBuilder;
use std::path::PathBuf;

pub struct Qwen25VLModelLoader {
    cache_dir: PathBuf,
}

impl Qwen25VLModelLoader {
    pub fn new() -> Result<Self> {
        let cache_dir = dirs::home_dir()
            .context("Failed to get home directory")?
            .join(".arkavo")
            .join("models")
            .join("qwen25vl");

        std::fs::create_dir_all(&cache_dir).context("Failed to create model cache directory")?;

        Ok(Self { cache_dir })
    }

    pub async fn load_model(&self, model_size: ModelSize) -> Result<ModelPaths> {
        let api = ApiBuilder::new()
            .with_progress(true)
            .build()
            .context("Failed to create HuggingFace API client")?;

        let (repo_id, model_file, mmproj_file) = match model_size {
            ModelSize::Small3B => (
                "Mungert/Qwen2.5-VL-3B-Instruct-GGUF",
                "Qwen2.5-VL-3B-Instruct-Q8_0.gguf",
                "Qwen2.5-VL-3B-Instruct-mmproj-f16.gguf",
            ),
            ModelSize::Medium7B => (
                "Mungert/Qwen2.5-VL-7B-Instruct-GGUF",
                "Qwen2.5-VL-7B-Instruct-Q8_0.gguf",
                "Qwen2.5-VL-7B-Instruct-mmproj-f16.gguf",
            ),
            ModelSize::Large32B => (
                "Mungert/Qwen2.5-VL-32B-Instruct-GGUF",
                "Qwen2.5-VL-32B-Instruct-Q8_0.gguf",
                "Qwen2.5-VL-32B-Instruct-mmproj-f16.gguf",
            ),
        };

        println!(
            "Downloading Qwen2.5-VL {} model from HuggingFace...",
            model_size.name()
        );
        println!("Repository: {}", repo_id);

        let repo = api.model(repo_id.to_string());

        println!("Downloading model file: {}", model_file);
        let model_path = repo
            .get(model_file)
            .await
            .with_context(|| format!("Failed to download model file: {}", model_file))?;

        println!("Downloading mmproj file: {}", mmproj_file);
        let mmproj_path = repo
            .get(mmproj_file)
            .await
            .with_context(|| format!("Failed to download mmproj file: {}", mmproj_file))?;

        println!("✓ Model downloaded successfully!");
        println!("  Model: {}", model_path.display());
        println!("  MMProj: {}", mmproj_path.display());

        Ok(ModelPaths {
            model: model_path,
            mmproj: mmproj_path,
        })
    }

    pub fn get_cached_paths(&self, model_size: ModelSize) -> Option<ModelPaths> {
        let (model_file, mmproj_file) = match model_size {
            ModelSize::Small3B => (
                "Qwen2.5-VL-3B-Instruct-Q8_0.gguf",
                "Qwen2.5-VL-3B-Instruct-mmproj-f16.gguf",
            ),
            ModelSize::Medium7B => (
                "Qwen2.5-VL-7B-Instruct-Q8_0.gguf",
                "Qwen2.5-VL-7B-Instruct-mmproj-f16.gguf",
            ),
            ModelSize::Large32B => (
                "Qwen2.5-VL-32B-Instruct-Q8_0.gguf",
                "Qwen2.5-VL-32B-Instruct-mmproj-f16.gguf",
            ),
        };

        let model_path = self.cache_dir.join(model_file);
        let mmproj_path = self.cache_dir.join(mmproj_file);

        if model_path.exists() && mmproj_path.exists() {
            Some(ModelPaths {
                model: model_path,
                mmproj: mmproj_path,
            })
        } else {
            None
        }
    }

    pub async fn ensure_model(&self, model_size: ModelSize) -> Result<ModelPaths> {
        if let Some(paths) = self.get_cached_paths(model_size) {
            println!("✓ Using cached model at: {}", paths.model.display());
            Ok(paths)
        } else {
            println!("Model not found in cache, downloading...");
            self.load_model(model_size).await
        }
    }
}

impl Default for Qwen25VLModelLoader {
    fn default() -> Self {
        Self::new().expect("Failed to create model loader")
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ModelSize {
    Small3B,
    Medium7B,
    Large32B,
}

impl ModelSize {
    fn name(&self) -> &'static str {
        match self {
            ModelSize::Small3B => "3B",
            ModelSize::Medium7B => "7B",
            ModelSize::Large32B => "32B",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelPaths {
    pub model: PathBuf,
    pub mmproj: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loader_creation() {
        let loader = Qwen25VLModelLoader::new();
        assert!(loader.is_ok());
    }

    #[test]
    fn test_cache_dir_created() {
        let loader = Qwen25VLModelLoader::new().unwrap();
        assert!(loader.cache_dir.exists());
    }
}
