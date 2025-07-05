use crate::{Error, Result};
use hf_hub::api::tokio::Api;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Model specification from models.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    pub name: String,
    pub description: String,
    pub hf_repo_id: String,
    pub hf_filename: String,
    pub sha256: String,
    pub size_gb: f32,
    pub context_length: usize,
    pub license: String,
}

/// Model manifest containing all available models
#[derive(Debug, Serialize, Deserialize)]
pub struct ModelManifest {
    pub models: Vec<ModelSpec>,
}

impl ModelManifest {
    /// Load manifest from embedded assets
    pub fn load() -> Result<Self> {
        let manifest_str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/models.toml"
        ));
        toml::from_str(manifest_str)
            .map_err(|e| Error::Config(format!("Failed to parse models.toml: {e}")))
    }

    /// Find a model by name
    pub fn find(&self, name: &str) -> Option<&ModelSpec> {
        self.models.iter().find(|m| m.name == name)
    }
}

/// Model downloader with progress and verification
pub struct ModelDownloader {
    api: Api,
}

impl ModelDownloader {
    /// Create a new downloader
    pub fn new() -> Result<Self> {
        // Create API builder
        let mut api_builder = hf_hub::api::tokio::ApiBuilder::new();

        // Set token if available from various sources
        let token = std::env::var("HF_TOKEN")
            .or_else(|_| std::env::var("HUGGING_FACE_HUB_TOKEN"))
            .ok();

        if let Some(ref token) = token {
            api_builder = api_builder.with_token(Some(token.clone()));
            tracing::info!("HuggingFace token configured");
        } else {
            tracing::info!("No HuggingFace token found, using anonymous access");
        }

        // Build the API
        let api = api_builder
            .build()
            .map_err(|e| Error::Model(format!("Failed to create HF API client: {e}")))?;

        Ok(Self { api })
    }

    /// Get the path where a model is stored in HuggingFace cache
    pub async fn get_model_path(&self, spec: &ModelSpec) -> Result<PathBuf> {
        // Try to get the model from cache without downloading
        let repo = self.api.model(spec.hf_repo_id.clone());
        match repo.get(&spec.hf_filename).await {
            Ok(path) => Ok(path),
            Err(_) => {
                // Model not in cache, need to download
                self.download(spec).await
            }
        }
    }

    /// Download a model with progress and verification
    pub async fn download(&self, spec: &ModelSpec) -> Result<PathBuf> {
        tracing::info!(
            "Downloading '{}' from repo '{}'...",
            spec.hf_filename,
            spec.hf_repo_id
        );

        // Use the hf-hub crate's built-in download method
        // This handles LFS, caching, and authentication automatically
        let repo = self.api.model(spec.hf_repo_id.clone());
        let hf_cache_path = repo
            .get(&spec.hf_filename)
            .await
            .map_err(|e| Error::Model(format!("Failed to download model: {e}")))?;

        // Verify the downloaded file's checksum
        if !self.verify_checksum(&hf_cache_path, &spec.sha256)? {
            return Err(Error::Model(
                "SHA256 mismatch for downloaded file".to_string(),
            ));
        }

        tracing::info!("Model {} successfully downloaded and verified", spec.name);
        Ok(hf_cache_path)
    }

    /// Verify SHA256 checksum of a file
    fn verify_checksum(&self, path: &Path, expected: &str) -> Result<bool> {
        let mut file = fs::File::open(path).map_err(Error::Io)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).map_err(Error::Io)?;

        let computed = format!("{:x}", hasher.finalize());
        Ok(computed == expected)
    }
}
