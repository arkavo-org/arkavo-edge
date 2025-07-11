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
    /// Base model repository containing tokenizer files
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_repo_id: Option<String>,
    /// Type of tokenizer (e.g., "json", "sentencepiece")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokenizer_type: Option<String>,
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

        // Download tokenizer files if base_repo_id is specified
        if let Some(base_repo_id) = &spec.base_repo_id {
            self.download_tokenizer(base_repo_id, &hf_cache_path, spec.tokenizer_type.as_deref())
                .await?;
        } else if spec.name.starts_with("gemma") {
            // Backwards compatibility for Gemma models
            self.download_gemma_tokenizer(&spec.hf_repo_id, &hf_cache_path)
                .await?;
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

    /// Download tokenizer files from base model repository
    ///
    /// # Panics
    ///
    /// Panics if the GGUF file path does not have a parent directory.
    pub async fn download_tokenizer(
        &self,
        base_repo_id: &str,
        gguf_path: &Path,
        tokenizer_type: Option<&str>,
    ) -> Result<()> {
        tracing::info!(
            "Downloading tokenizer from base repository: {}",
            base_repo_id
        );

        let repo = self.api.model(base_repo_id.to_string());
        let gguf_dir = gguf_path
            .parent()
            .expect("GGUF file must have a parent directory");

        // Try to download tokenizer.json first (preferred format)
        let tokenizer_downloaded = match repo.get("tokenizer.json").await {
            Ok(path) => {
                let target_path = gguf_dir.join("tokenizer.json");
                if !target_path.exists() {
                    fs::copy(&path, &target_path).map_err(Error::Io)?;
                    tracing::info!("Copied tokenizer.json to GGUF directory: {:?}", target_path);
                } else {
                    tracing::info!("Tokenizer.json already exists at: {:?}", target_path);
                }
                true
            }
            Err(e) => {
                tracing::debug!("tokenizer.json not found: {}", e);
                false
            }
        };

        // If tokenizer.json wasn't found and tokenizer_type suggests sentencepiece, try tokenizer.model
        if !tokenizer_downloaded && matches!(tokenizer_type, Some("sentencepiece") | None) {
            match repo.get("tokenizer.model").await {
                Ok(path) => {
                    let target_path = gguf_dir.join("tokenizer.model");
                    if !target_path.exists() {
                        fs::copy(&path, &target_path).map_err(Error::Io)?;
                        tracing::info!(
                            "Copied tokenizer.model to GGUF directory: {:?}",
                            target_path
                        );
                    } else {
                        tracing::info!("Tokenizer.model already exists at: {:?}", target_path);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to download tokenizer files from {}: {}. Make sure you're authenticated if the model requires it.",
                        base_repo_id,
                        e
                    );
                    return Ok(()); // Don't fail the whole download if tokenizer is missing
                }
            }
        }

        // Download tokenizer_config.json if available
        if let Ok(config_path) = repo.get("tokenizer_config.json").await {
            let target_config = gguf_dir.join("tokenizer_config.json");
            if !target_config.exists() {
                fs::copy(&config_path, &target_config).map_err(Error::Io)?;
                tracing::info!("Copied tokenizer_config.json to GGUF directory");
            }
        }

        Ok(())
    }

    /// Download tokenizer files for Gemma models (backwards compatibility)
    async fn download_gemma_tokenizer(&self, gguf_repo_id: &str, gguf_path: &Path) -> Result<()> {
        // Map GGUF repo to base model repo
        let base_repo = match gguf_repo_id {
            "google/gemma-3-1b-it-qat-q4_0-gguf" => "google/gemma-3-1b-it",
            "unsloth/gemma-3n-E4B-it-GGUF" => "google/gemma-2b-it", // Gemma 3n uses Gemma 2 tokenizer
            _ => {
                tracing::warn!(
                    "Unknown GGUF repo, skipping tokenizer download: {}",
                    gguf_repo_id
                );
                return Ok(());
            }
        };

        tracing::info!("Downloading tokenizer from base repository: {}", base_repo);

        // Download tokenizer.json file (JSON format that tokenizers library expects)
        let repo = self.api.model(base_repo.to_string());
        let tokenizer_path = match repo.get("tokenizer.json").await {
            Ok(path) => path,
            Err(e) => {
                tracing::warn!(
                    "Failed to download tokenizer.json: {}. Make sure you're authenticated with 'huggingface-cli login' and have accepted the Gemma license.",
                    e
                );
                return Ok(()); // Don't fail the whole download if tokenizer is missing
            }
        };

        // Copy tokenizer to the same directory as the GGUF file
        let gguf_dir = gguf_path
            .parent()
            .expect("GGUF file must have a parent directory");

        let target_path = gguf_dir.join("tokenizer.json");
        if !target_path.exists() {
            fs::copy(&tokenizer_path, &target_path).map_err(Error::Io)?;
            tracing::info!("Copied tokenizer.json to GGUF directory: {:?}", target_path);
        } else {
            tracing::info!("Tokenizer already exists at: {:?}", target_path);
        }

        // Optionally download tokenizer_config.json (small file, useful for configuration)
        if let Ok(config_path) = repo.get("tokenizer_config.json").await {
            let target_config = gguf_dir.join("tokenizer_config.json");
            if !target_config.exists() {
                fs::copy(&config_path, &target_config).map_err(Error::Io)?;
                tracing::info!("Copied tokenizer_config.json to GGUF directory");
            }
        }

        Ok(())
    }
}
