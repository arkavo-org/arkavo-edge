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

pub async fn run(cmd: &ModelCommand) -> Result<()> {

    match &cmd.command {
        ModelSubcommand::List => {
            #[cfg(feature = "local")]
            {
                use arkavo_llm::local::{ModelDownloader, ModelManifest};

                // Load manifest to show available models
                let manifest = ModelManifest::load()?;
                let downloader = ModelDownloader::new()?;

                println!("Available models:");

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
                    let default_marker = if spec.name == "gemma3-1b-it-qat" { " (default)" } else { "" };

                    println!(
                        "  {} {} - {} ({:.1} GB){}",
                        status, spec.name, spec.description, spec.size_gb, default_marker
                    );
                }

                println!("\nUse 'arkavo model download <name>' to download a model.");
            }

            #[cfg(not(feature = "local"))]
            {
                println!("Model management requires the 'local' feature to be enabled.");
            }
        }

        ModelSubcommand::Switch { name } => {
            // For future multi-model support
            println!("Model switching is not yet implemented. The default model (gemma3-1b-it-qat) will be used if available.");
            let _ = name; // Suppress unused warning
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

                // Model is now available in HF cache
            }
        }

        ModelSubcommand::Add { path, name } => {
            // For future extensibility
            println!("Manual model addition is not yet implemented. Please download models using 'arkavo model download'.");
            let _ = (path, name); // Suppress unused warnings
        }
    }

    Ok(())
}
