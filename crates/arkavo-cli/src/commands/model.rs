#![allow(clippy::collapsible_if)]

use anyhow::Result;
use clap::{Args, Subcommand};
use std::collections::HashMap;
use std::path::PathBuf;

/// Check if a model has proper chat template support
fn get_model_compatibility(model_name: &str) -> (&'static str, &'static str) {
    let name_lower = model_name.to_lowercase();
    if name_lower.contains("qwen") {
        ("compatible", "Qwen3")
    } else if name_lower.contains("mistral") || name_lower.contains("ministral") {
        ("compatible", "MistralV3")
    } else if name_lower.contains("gemma") {
        ("compatible", "Gemma3")
    } else {
        ("incompatible", "unknown format")
    }
}

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

/// Parse .arkavo/AGENTS.md to extract remote model configuration and API keys
fn parse_agents_config() -> HashMap<String, Vec<String>> {
    let mut models = HashMap::new();

    // Try to find .arkavo/AGENTS.md in current directory or parent directories
    let mut current_dir = std::env::current_dir().ok();
    let mut agents_file = None;

    while let Some(dir) = current_dir {
        let candidate = dir.join(".arkavo").join("AGENTS.md");
        if candidate.exists() {
            agents_file = Some(candidate);
            break;
        }
        current_dir = dir.parent().map(|p| p.to_path_buf());
    }

    if let Some(path) = agents_file {
        if let Ok(content) = std::fs::read_to_string(path) {
            let mut current_provider = String::new();

            for line in content.lines() {
                let trimmed = line.trim();

                // Look for provider headers (e.g., "## Gemini Models", "## Local Models")
                if trimmed.starts_with("##") {
                    current_provider = trimmed.trim_start_matches("##").trim().to_string();
                    models
                        .entry(current_provider.clone())
                        .or_insert_with(Vec::new);
                }
                // Look for API key assignments and set them as environment variables
                else if trimmed.contains("API_KEY=") {
                    if let Some((key, value)) = trimmed.split_once('=') {
                        let key = key.trim();
                        let value = value.trim();
                        // Only set if not already set in environment
                        if std::env::var(key).is_err() {
                            // SAFETY: We're setting environment variables during config parsing
                            // This is safe as long as called before multi-threading
                            unsafe {
                                std::env::set_var(key, value);
                            }
                        }
                    }
                }
                // Look for model entries (lines starting with -)
                else if trimmed.starts_with('-') && !current_provider.is_empty() {
                    let model = trimmed
                        .trim_start_matches('-')
                        .trim()
                        .split(':')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();

                    if !model.is_empty() {
                        models
                            .entry(current_provider.clone())
                            .or_insert_with(Vec::new)
                            .push(model);
                    }
                }
            }
        }
    }

    models
}

/// List all available GGUF models using the model_discovery module
fn list_local_gguf_models() -> Vec<(String, String, PathBuf, u64)> {
    let mut found_models = Vec::new();

    // Get HF cache directory
    let hf_cache_dir = if let Ok(hf_home) = std::env::var("HF_HOME") {
        Some(PathBuf::from(hf_home).join("hub"))
    } else {
        dirs::home_dir().map(|d| d.join(".cache").join("huggingface").join("hub"))
    };

    if let Some(cache_dir) = hf_cache_dir {
        if cache_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&cache_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                        if dir_name.starts_with("models--") {
                            let snapshots_dir = path.join("snapshots");
                            if snapshots_dir.exists() {
                                if let Ok(snapshot_entries) = std::fs::read_dir(&snapshots_dir) {
                                    for snapshot in snapshot_entries.flatten() {
                                        let snapshot_path = snapshot.path();
                                        if snapshot_path.is_dir() {
                                            if let Ok(files) = std::fs::read_dir(&snapshot_path) {
                                                for file in files.flatten() {
                                                    if let Some(name) = file.file_name().to_str() {
                                                        if name.ends_with(".gguf") {
                                                            let model_name = dir_name
                                                                .strip_prefix("models--")
                                                                .unwrap_or(dir_name)
                                                                .replace("--", "/");
                                                            let file_path = file.path();
                                                            let size =
                                                                std::fs::metadata(&file_path)
                                                                    .map(|m| m.len())
                                                                    .unwrap_or(0);
                                                            found_models.push((
                                                                model_name,
                                                                name.to_string(),
                                                                file_path,
                                                                size,
                                                            ));
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
        }
    }

    found_models
}

#[allow(clippy::unused_async)]
pub async fn run(cmd: &ModelCommand) -> Result<()> {
    match &cmd.command {
        ModelSubcommand::List => {
            println!("Available Models\n");

            // Read .arkavo/AGENTS.md configuration
            let agents_config = parse_agents_config();

            // Show preferred models from .arkavo/AGENTS.md
            if !agents_config.is_empty() {
                println!("Preferred Models (from .arkavo/AGENTS.md):");
                for (provider, models) in &agents_config {
                    println!("\n  {provider}:");
                    for model in models {
                        println!("    • {model}");
                    }
                }
                println!();
            } else {
                // Fallback: check for Gemini
                println!("Preferred Models:");
                if std::env::var("GEMINI_API_KEY").is_ok() {
                    println!("  ✓ Gemini API configured");
                } else {
                    println!("  ✗ No API keys configured");
                    println!("  Set GEMINI_API_KEY to use Gemini models");
                }
                println!("\n  Configure preferred models in .arkavo/AGENTS.md");
                println!();
            }

            // Show local GGUF models
            println!("Local Models (GGUF via llama.cpp):");
            let found_models = list_local_gguf_models();

            if !found_models.is_empty() {
                for (model_name, file_name, path, size) in &found_models {
                    let size_gb = *size as f64 / (1024.0 * 1024.0 * 1024.0);
                    let (compat_status, format) = get_model_compatibility(model_name);
                    let icon = if compat_status == "compatible" {
                        "✓"
                    } else {
                        "⚠"
                    };
                    println!("  {icon} {model_name}/{file_name} ({size_gb:.1} GB) [{format}]");
                    if compat_status == "incompatible" {
                        println!("      Warning: May use incorrect chat template");
                    }
                    if std::env::var("ARKAVO_DEBUG").is_ok() {
                        println!("    Path: {}", path.display());
                    }
                }
                println!("\nDownload models with: hf download <repo> <file.gguf>");
            } else {
                println!("  No GGUF models found in HuggingFace cache");
                println!("  Download with: hf download Qwen/Qwen3-0.6B-GGUF Qwen3-0.6B-Q8_0.gguf");
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
            // Model downloading was removed with the local feature
            // Users should download models manually from HuggingFace
            let _ = name; // Suppress unused warning
            anyhow::bail!(
                "Model downloading has been removed. Please download GGUF models manually from HuggingFace."
            );
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
