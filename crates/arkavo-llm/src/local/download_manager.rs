use crate::{Error, Result};
use hf_hub::api::tokio::Api;
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

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
    cache_dir: PathBuf,
    api: Api,
}

impl ModelDownloader {
    /// Create a new downloader with default cache directory
    pub fn new() -> Result<Self> {
        let cache_dir = Self::default_cache_dir()?;
        let api =
            Api::new().map_err(|e| Error::Model(format!("Failed to create HF API client: {e}")))?;

        Ok(Self { cache_dir, api })
    }

    /// Get the default cache directory (~/.cache/arkavo/models)
    fn default_cache_dir() -> Result<PathBuf> {
        let home = dirs::home_dir()
            .ok_or_else(|| Error::Config("Could not determine home directory".to_string()))?;
        let cache_dir = home.join(".cache").join("arkavo").join("models");
        Ok(cache_dir)
    }

    /// Get the path where a model would be stored
    pub fn get_model_path(&self, spec: &ModelSpec) -> PathBuf {
        self.cache_dir.join(&spec.hf_filename)
    }

    /// Check if a model is already downloaded and verified
    pub fn is_downloaded(&self, spec: &ModelSpec) -> bool {
        let model_path = self.get_model_path(spec);
        if !model_path.exists() {
            return false;
        }

        // Verify checksum
        self.verify_checksum(&model_path, &spec.sha256)
            .unwrap_or_default()
    }

    /// Download a model with progress and verification
    /// Download a model with progress and verification
    ///
    /// # Panics
    ///
    /// Panics if the progress bar template is invalid (should never happen with hardcoded template)
    pub async fn download(&self, spec: &ModelSpec) -> Result<PathBuf> {
        let model_path = self.get_model_path(spec);

        // Check if already downloaded
        if self.is_downloaded(spec) {
            tracing::info!("Model {} already downloaded and verified", spec.name);
            return Ok(model_path);
        }

        // Ensure cache directory exists
        fs::create_dir_all(&self.cache_dir).map_err(Error::Io)?;

        // Set up progress bar
        let pb = ProgressBar::new((spec.size_gb * 1024.0 * 1024.0 * 1024.0) as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        pb.set_message(format!("Downloading {}", spec.name));

        // Download to temporary file
        let temp_path = self.cache_dir.join(format!("{}.partial", spec.hf_filename));

        // Download using async API
        let repo = self.api.model(spec.hf_repo_id.clone());

        // Create temp file for writing
        let mut file = tokio::fs::File::create(&temp_path)
            .await
            .map_err(Error::Io)?;

        // Set up cleanup guard
        let guard = PartialFileGuard::new(temp_path.clone());

        // Download the file
        let download_url = repo.url(&spec.hf_filename);

        let response = reqwest::get(&download_url)
            .await
            .map_err(|e| Error::Model(format!("Failed to start download: {e}")))?;

        if !response.status().is_success() {
            return Err(Error::Model(format!(
                "Download failed with status: {}",
                response.status()
            )));
        }

        // Set up SHA256 hasher
        let mut hasher = Sha256::new();
        let mut bytes_written = 0u64;

        // Stream the download
        let mut stream = response.bytes_stream();
        use futures::StreamExt;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| Error::Model(format!("Download chunk failed: {e}")))?;

            // Update progress
            bytes_written += chunk.len() as u64;
            pb.set_position(bytes_written);

            // Update hash
            hasher.update(&chunk);

            // Write to file
            file.write_all(&chunk).await.map_err(Error::Io)?;
        }

        // Finalize the file
        file.flush().await.map_err(Error::Io)?;
        drop(file);

        // Verify checksum
        let computed_hash = format!("{:x}", hasher.finalize());
        if computed_hash != spec.sha256 {
            return Err(Error::Model(format!(
                "SHA256 mismatch: expected {}, got {}",
                spec.sha256, computed_hash
            )));
        }

        pb.finish_with_message("Download complete");

        // Disarm the cleanup guard since download succeeded
        guard.disarm();

        // Atomic rename to final location
        fs::rename(&temp_path, &model_path).map_err(Error::Io)?;

        // Write Notice.txt for Gemma models
        if spec.license == "gemma" {
            self.write_gemma_notice(&spec)?;
        }

        tracing::info!("Successfully downloaded and verified {}", spec.name);
        Ok(model_path)
    }

    /// Verify SHA256 checksum of a file
    fn verify_checksum(&self, path: &Path, expected: &str) -> Result<bool> {
        let mut file = fs::File::open(path).map_err(Error::Io)?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher).map_err(Error::Io)?;

        let computed = format!("{:x}", hasher.finalize());
        Ok(computed == expected)
    }

    /// Write Notice.txt for Gemma models
    fn write_gemma_notice(&self, spec: &ModelSpec) -> Result<()> {
        let notice_path = self
            .cache_dir
            .join(format!("{}.Notice.txt", spec.hf_filename));
        let notice_content = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../THIRD-PARTY-LICENSES.md"
        ));

        // Extract Gemma section from THIRD-PARTY-LICENSES.md
        let gemma_section = notice_content
            .split("## Gemma")
            .nth(1)
            .and_then(|s| s.split("##").next())
            .unwrap_or(
                "See https://www.kaggle.com/models/google/gemma/license/consent for license terms.",
            );

        fs::write(
            &notice_path,
            format!("Gemma Model License\n\n{}", gemma_section.trim()),
        )
        .map_err(Error::Io)?;

        Ok(())
    }
}

/// Cleanup guard to remove partial files on drop
pub struct PartialFileGuard {
    path: Option<PathBuf>,
}

impl PartialFileGuard {
    pub fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub fn disarm(mut self) {
        self.path = None;
    }
}

impl Drop for PartialFileGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = fs::remove_file(path);
        }
    }
}
