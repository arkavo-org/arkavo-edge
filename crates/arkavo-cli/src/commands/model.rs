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
    } else if name_lower.contains("glm-4") || name_lower.contains("glm4") {
        ("compatible", "GLM4")
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
                println!("\nDownload more models with: arkavo model download <name>");
            } else {
                println!("  No GGUF models found in HuggingFace cache");
                println!("  Download with: arkavo model download");
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
            use crate::first_run::{RecommendedModel, detect_capabilities, download_model};

            let caps = detect_capabilities();

            let model = match name.as_deref() {
                Some("gemma-4-e2b" | "gemma4-e2b" | "gemma-e2b") => RecommendedModel::Gemma4E2B,
                Some("gemma-4-e4b" | "gemma4-e4b" | "gemma-e4b") => RecommendedModel::Gemma4E4B,
                Some("gemma-4-12b" | "gemma4-12b" | "gemma-12b" | "gemma") => {
                    RecommendedModel::Gemma4_12B
                }
                Some("qwen3.5-0.8b" | "qwen3-0.6b" | "qwen" | "qwen3") => {
                    RecommendedModel::Qwen35_0_8B
                }
                Some("ministral-3b" | "ministral3b" | "ministral") => RecommendedModel::Ministral3B,
                Some("ministral-8b" | "ministral8b") => RecommendedModel::Ministral8B,
                Some("glm-4.7-flash" | "glm" | "glm4") => RecommendedModel::Glm47Flash,
                Some(other) => {
                    println!("Unknown model: {other}");
                    println!();
                    println!("Available models:");
                    println!(
                        "  gemma-4-e2b   - Gemma 4 E2B (~2.9 GB) - Default small, fast routing"
                    );
                    println!(
                        "  gemma-4-12b   - Gemma 4 12B (~6.9 GB) - Default medium, most capable"
                    );
                    println!("  gemma-4-e4b   - Gemma 4 E4B (~5 GB) - Edge medium");
                    println!("  qwen3.5-0.8b  - Qwen3.5 0.8B (~550 MB) - Best for embedded");
                    println!("  ministral-3b  - Ministral 3B (~2.5 GB)");
                    println!("  ministral-8b  - Ministral 8B (~5.5 GB)");
                    println!(
                        "  glm-4.7-flash - GLM-4.7-Flash (~18 GB) - 30B MoE, requires 32GB+ RAM"
                    );
                    return Ok(());
                }
                None => {
                    println!(
                        "No model specified, using recommended: {}",
                        caps.recommended_model.display_name()
                    );
                    caps.recommended_model
                }
            };

            // Check system capabilities for GLM-4.7-Flash
            if matches!(model, RecommendedModel::Glm47Flash) {
                use crate::first_run::DeviceProfile;
                println!(
                    "System: {} ({} GB RAM)",
                    caps.device_profile, caps.total_ram_gb
                );
                match caps.device_profile {
                    DeviceProfile::Workstation | DeviceProfile::HighMemoryWorkstation => {
                        println!("System meets GLM-4.7-Flash requirements.");
                    }
                    _ => {
                        println!();
                        println!(
                            "Warning: GLM-4.7-Flash requires 32GB+ RAM for reasonable performance."
                        );
                        println!(
                            "Your system has {} GB RAM ({}).",
                            caps.total_ram_gb, caps.device_profile
                        );
                        println!();
                        println!("Recommended alternatives:");
                        println!(
                            "  arkavo model download ministral-8b  (5.5 GB, works on 16GB+ RAM)"
                        );
                        println!(
                            "  arkavo model download ministral-3b  (2.5 GB, works on 8GB+ RAM)"
                        );
                        println!();
                        print!("Continue anyway? (y/N) ");
                        use std::io::{self, Write};
                        let _ = io::stdout().flush();
                        let mut input = String::new();
                        if io::stdin().read_line(&mut input).is_err() {
                            return Ok(());
                        }
                        let input = input.trim().to_lowercase();
                        if input != "y" && input != "yes" {
                            println!("Download cancelled.");
                            return Ok(());
                        }
                    }
                }
            }

            println!(
                "Downloading {} ({:.1} GB)...",
                model.display_name(),
                model.size_bytes() as f64 / 1_000_000_000.0
            );
            download_model(&model)
                .await
                .map_err(|e| anyhow::anyhow!(e))?;
            println!("\nModel ready! Run 'arkavo' to start.");
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
