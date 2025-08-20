use anyhow::Result;
use clap::{Args, Subcommand};
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

// Removed ModelFormat, ModelMeta, and ModelRegistry structs
// These are no longer needed with the simplified zero-config approach

#[allow(clippy::unused_async)]
pub async fn run(cmd: &ModelCommand) -> Result<()> {
    match &cmd.command {
        ModelSubcommand::List => {
            // Always show remote models
            println!("Available models:\n");
            
            // Check for Ollama
            println!("Remote Models (Ollama):");
            if let Ok(client) = arkavo_llm::LlmClient::from_env() {
                println!("  ✓ Ollama server available at {}", client.provider_name());
                println!("  Run 'ollama list' to see available models");
                println!("  Run 'ollama pull <model>' to download models");
            } else {
                println!("  ✗ Ollama not running (start with 'ollama serve')");
            }
            
            println!();
            
            // Check for local GGUF models in HF cache
            println!("Local Models (GGUF via llama.cpp):");
            
            // Check HuggingFace cache for GGUF models
            let hf_cache_dir = dirs::home_dir()
                .map(|d| d.join(".cache/huggingface/hub"));
            
            if let Some(cache_dir) = hf_cache_dir {
                if cache_dir.exists() {
                    let mut found_models = Vec::new();
                    
                    // Scan for GGUF models in the cache
                    if let Ok(entries) = std::fs::read_dir(&cache_dir) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_dir() {
                                let dir_name = path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or("");
                                
                                // Check if it's a model directory and contains GGUF files
                                if dir_name.starts_with("models--") {
                                    // Look for GGUF files in snapshots
                                    let snapshots_dir = path.join("snapshots");
                                    if snapshots_dir.exists() {
                                        if let Ok(snapshot_entries) = std::fs::read_dir(&snapshots_dir) {
                                            for snapshot in snapshot_entries.flatten() {
                                                let snapshot_path = snapshot.path();
                                                if snapshot_path.is_dir() {
                                                    // Check for .gguf files
                                                    if let Ok(files) = std::fs::read_dir(&snapshot_path) {
                                                        for file in files.flatten() {
                                                            if let Some(name) = file.file_name().to_str() {
                                                                if name.ends_with(".gguf") {
                                                                    let model_name = dir_name.strip_prefix("models--")
                                                                        .unwrap_or(dir_name)
                                                                        .replace("--", "/");
                                                                    let file_path = file.path();
                                                                    let size = std::fs::metadata(&file_path)
                                                                        .map(|m| m.len())
                                                                        .unwrap_or(0);
                                                                    found_models.push((model_name, name.to_string(), file_path, size));
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    if !found_models.is_empty() {
                        for (model_name, file_name, path, size) in &found_models {
                            let size_gb = *size as f64 / (1024.0 * 1024.0 * 1024.0);
                            println!("  ✓ {}/{} ({:.1} GB)", model_name, file_name, size_gb);
                            println!("    Path: {}", path.display());
                        }
                        
                        println!("\nUse 'arkavo chat --model <path-to-gguf>' to use a local model");
                        println!("Or set ARKAVO_MODEL_PATH environment variable");
                    } else {
                        println!("  No GGUF models found in HuggingFace cache");
                        println!("  Download with: huggingface-cli download <repo> <file.gguf>");
                    }
                } else {
                    println!("  HuggingFace cache directory not found");
                }
            } else {
                println!("  Could not determine HuggingFace cache location");
            }
            
            // Show local models if feature is enabled
            #[cfg(feature = "local")]
            {
                use arkavo_llm::local::{ModelDownloader, ModelManifest};

                println!("\nLocal Models (arkavo managed):");
                
                // Load manifest to show available models
                match ModelManifest::load() {
                    Ok(manifest) => {
                        let downloader = ModelDownloader::new()?;
                        
                        // Show manifest models with download status
                        for spec in &manifest.models {
                            // Check if model exists in HF cache
                            let downloaded = downloader.get_model_path(spec).await.is_ok();

                            let status = if downloaded {
                                "✓ downloaded"
                            } else {
                                "  available"
                            };

                            // Mark default model
                            let default_marker = if spec.name == "gemma3-1b-it-qat" {
                                " (default)"
                            } else {
                                ""
                            };

                            println!(
                                "  {} {} - {} ({:.1} GB){}",
                                status, spec.name, spec.description, spec.size_gb, default_marker
                            );
                        }
                        
                        println!("\nUse 'arkavo model download <name>' to download a local model.");
                    }
                    Err(_) => {
                        println!("  Local model manifest not available");
                    }
                }
            }
        }

        ModelSubcommand::Switch { name } => {
            // For future multi-model support
            println!(
                "Model switching is not yet implemented. The default model (gemma3-1b-it-qat) will be used if available."
            );
            let _ = name; // Suppress unused warning
        }

        ModelSubcommand::Download { name } => {
            // Check if 'local' feature is enabled
            #[cfg(not(feature = "local"))]
            {
                let _ = name; // Suppress unused warning
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

                println!("Downloading model '{model_name}'...");

                // Download the model (or get from cache if already downloaded)
                let model_path = downloader.download(spec).await?;

                println!("Model '{model_name}' ready at: {}", model_path.display());

                // Model is now available in HF cache
            }
        }

        ModelSubcommand::Add { path, name } => {
            // For future extensibility
            println!(
                "Manual model addition is not yet implemented. Please download models using 'arkavo model download'."
            );
            let _ = (path, name); // Suppress unused warnings
        }
    }

    Ok(())
}
