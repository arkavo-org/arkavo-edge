use anyhow::Result;
use arkavo_memory::storage::MemoryStorage;
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Args)]
pub struct ModelCommand {
    #[command(subcommand)]
    command: ModelSubcommand,
}

#[derive(Subcommand)]
enum ModelSubcommand {
    /// List available models
    List,

    /// Switch to a different model
    Switch {
        /// Name of the model to switch to
        name: String,
    },

    /// Download a model from the registry
    Download {
        /// Name of the model to download (defaults to gemma3-1b-it-qat)
        name: Option<String>,
    },

    /// Add a local model file
    Add {
        /// Path to the model file
        path: PathBuf,

        /// Name to give the model
        #[arg(long)]
        name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModelFormat {
    #[serde(rename = "gguf")]
    Gguf,
    #[serde(rename = "safetensors")]
    Safetensors,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelMeta {
    /// Model identifier (e.g., "gemma3n-e2b")
    pub name: String,

    /// Model format
    pub format: ModelFormat,

    /// Context length in tokens
    pub context_length: usize,

    /// Hugging Face repository ID (e.g., "google/gemma-3n-2b")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hf_repo_id: Option<String>,

    /// SHA256 checksum of the model file
    pub sha256: String,

    /// Model size in gigabytes
    pub size_gb: f32,

    /// Local path to the model file (if downloaded)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_path: Option<PathBuf>,

    /// Whether this is the currently active model
    #[serde(default)]
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelRegistry {
    /// List of available models
    pub models: Vec<ModelMeta>,
}

const MODELS_LOCAL_KEY: &str = "models.local";

impl ModelRegistry {
    /// Load registry from memory storage
    async fn load_from_storage(storage: &MemoryStorage) -> Result<Self> {
        // Search for the models.local entry
        let results = storage.search(MODELS_LOCAL_KEY, 1, Some("config")).await?;

        if let Some(result) = results.first() {
            match serde_json::from_str::<ModelRegistry>(&result.memory.content) {
                Ok(registry) => Ok(registry),
                Err(_) => Ok(Self::default()),
            }
        } else {
            Ok(Self::default())
        }
    }

    /// Save registry to memory storage
    async fn save_to_storage(&self, storage: &MemoryStorage) -> Result<()> {
        let content = serde_json::to_string(self)?;

        // Create a memory entry for the registry
        let memory = arkavo_memory::models::Memory {
            id: uuid::Uuid::new_v4(),
            content,
            embedding: vec![], // No embeddings needed for config
            category: Some("config".to_string()),
            metadata: Some(serde_json::json!({
                "key": MODELS_LOCAL_KEY,
                "type": "model_registry",
                "version": "1.0"
            })),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        storage.store(memory).await?;
        Ok(())
    }
}

pub async fn run(cmd: &ModelCommand) -> Result<()> {
    let storage = MemoryStorage::new().await?;
    let mut registry = ModelRegistry::load_from_storage(&storage).await?;

    match &cmd.command {
        ModelSubcommand::List => {
            #[cfg(feature = "local")]
            {
                use arkavo_llm::local::ModelManifest;

                // Load manifest to show available models
                let manifest = ModelManifest::load()?;

                println!("Available models:");

                // Show manifest models with download status
                for spec in &manifest.models {
                    // Check if in registry
                    let registry_model = registry.models.iter().find(|m| m.name == spec.name);
                    let active = registry_model.map_or(false, |m| m.is_active);
                    let downloaded = registry_model.and_then(|m| m.local_path.as_ref()).is_some();

                    let status = if downloaded {
                        "✓ downloaded"
                    } else {
                        "  available"
                    };
                    let active_marker = if active { " (active)" } else { "" };

                    println!(
                        "  {} {} - {} ({:.1} GB){}",
                        status, spec.name, spec.description, spec.size_gb, active_marker
                    );
                }

                // Show any additional local models not in manifest
                for model in &registry.models {
                    if !manifest.models.iter().any(|s| s.name == model.name) {
                        let active = if model.is_active { " (active)" } else { "" };
                        println!(
                            "  ✓ {} - local ({:.1} GB){}",
                            model.name, model.size_gb, active
                        );
                    }
                }

                if registry.models.is_empty() {
                    println!("\nUse 'arkavo model download <name>' to download a model.");
                }
            }

            #[cfg(not(feature = "local"))]
            {
                if registry.models.is_empty() {
                    println!("No models available. Use 'arkavo model add' to add models.");
                } else {
                    println!("Available models:");
                    for model in &registry.models {
                        let active = if model.is_active { " (active)" } else { "" };
                        let status = if model.local_path.is_some() {
                            "downloaded"
                        } else {
                            "available"
                        };
                        println!(
                            "  {} - {} ({:.1} GB, {}){}",
                            model.name, status, model.size_gb, model.context_length, active
                        );
                    }
                }
            }
        }

        ModelSubcommand::Switch { name } => {
            let mut found = false;
            for model in &mut registry.models {
                if model.name == *name {
                    model.is_active = true;
                    found = true;
                    println!("Switched to model: {}", name);
                } else {
                    model.is_active = false;
                }
            }

            if !found {
                anyhow::bail!("Model '{}' not found", name);
            }

            registry.save_to_storage(&storage).await?;
        }

        ModelSubcommand::Download { name } => {
            // Check if 'local' feature is enabled
            #[cfg(not(feature = "local"))]
            {
                anyhow::bail!("Model downloading requires the 'local' feature to be enabled");
            }

            #[cfg(feature = "local")]
            {
                use arkavo_llm::local::{ModelDownloader, ModelManifest};

                // Default to gemma3-1b-it-qat if no name provided
                let model_name = name.as_deref().unwrap_or("gemma3-1b-it-qat");

                // Load model manifest
                let manifest = ModelManifest::load()?;

                // Find the model spec
                let spec = manifest.find(model_name).ok_or_else(|| {
                    anyhow::anyhow!("Model '{}' not found in manifest", model_name)
                })?;

                // Create downloader
                let downloader = ModelDownloader::new()?;

                println!("Downloading model '{}'...", model_name);

                // Download the model (or get from cache if already downloaded)
                let model_path = downloader.download(spec).await?;

                println!("Model '{}' ready at: {:?}", model_name, model_path);

                // Check if already in registry and update or add
                if let Some(existing) = registry.models.iter_mut().find(|m| m.name == spec.name) {
                    // Update existing entry
                    existing.local_path = Some(model_path);
                } else {
                    // Add new entry
                    let model = ModelMeta {
                        name: spec.name.clone(),
                        format: ModelFormat::Gguf,
                        context_length: spec.context_length,
                        hf_repo_id: Some(spec.hf_repo_id.clone()),
                        sha256: spec.sha256.clone(),
                        size_gb: spec.size_gb,
                        local_path: Some(model_path),
                        is_active: registry.models.is_empty(),
                    };
                    registry.models.push(model);
                }

                registry.save_to_storage(&storage).await?;
            }
        }

        ModelSubcommand::Add { path, name } => {
            if !path.exists() {
                anyhow::bail!("Model file not found: {}", path.display());
            }

            // TODO: Calculate SHA256, detect format, etc.
            let model = ModelMeta {
                name: name.clone(),
                format: ModelFormat::Gguf, // TODO: Detect from file
                context_length: 8192,      // TODO: Read from model metadata
                hf_repo_id: None,
                sha256: "placeholder".to_string(), // TODO: Calculate
                size_gb: 0.0,                      // TODO: Calculate from file size
                local_path: Some(path.clone()),
                is_active: registry.models.is_empty(), // Make active if first model
            };

            registry.models.push(model);
            registry.save_to_storage(&storage).await?;

            println!("Added model '{}' from {}", name, path.display());
        }
    }

    Ok(())
}
