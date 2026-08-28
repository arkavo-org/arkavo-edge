#![allow(clippy::collapsible_if)]

use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::PathBuf;

use super::model_list::{get_model_compatibility, list_local_gguf_models, parse_agents_config};

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

    /// Wrap a GGUF into a KAS-gated .gguf.tdf archive
    Protect {
        /// Path to the source .gguf file
        path: PathBuf,

        /// Output archive path (default: <source>.tdf)
        #[arg(long)]
        output: Option<PathBuf>,

        /// KAS base URL to wrap the payload key to
        #[arg(long)]
        kas_url: Option<String>,

        /// Maximum plaintext bytes per weight segment (default 4 MiB)
        #[arg(long)]
        max_segment: Option<u64>,

        /// Policy data attribute FQN; repeatable
        #[arg(long = "attribute")]
        attributes: Vec<String>,

        /// Delete the plaintext source after a successful wrap
        #[arg(long)]
        delete_source: bool,
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

        ModelSubcommand::Protect {
            path,
            output,
            kas_url,
            max_segment,
            attributes,
            delete_source,
        } => {
            super::model_protect::run(super::model_protect::ProtectArgs {
                path,
                output: output.as_deref(),
                kas_url: kas_url.as_deref(),
                max_segment: *max_segment,
                attributes,
                delete_source: *delete_source,
            })
            .await?;
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
