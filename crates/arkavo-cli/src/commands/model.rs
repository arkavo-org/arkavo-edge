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
        /// Name of the model to download
        name: String,
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
            if registry.models.is_empty() {
                println!(
                    "No models available. Use 'arkavo model download' or 'arkavo model add' to add models."
                );
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
            // TODO: Implement model downloading
            println!("Downloading model '{}' (not yet implemented)", name);
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
