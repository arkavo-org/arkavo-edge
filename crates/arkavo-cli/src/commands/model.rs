#![allow(clippy::collapsible_if)]

use crate::commands::kit::kit_model_to_hint;
use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

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

/// Preferred models declared by the discovered SwarmKit kit: one `(role id,
/// model hint)` pair per role whose `agent_provisioning.model` maps to a
/// known local-edge model via [`kit_model_to_hint`]. Returns `None` when no
/// kit is discovered, or when a kit exists but declares no recognized local
/// models — the caller falls back to the same env-key status display either
/// way, matching how the old AGENTS.md-based lookup handled "nothing
/// configured".
fn kit_preferred_models(cwd: &Path) -> Option<Vec<(String, String)>> {
    let discovered = arkavo_swarmkit::load_discovered_kit(cwd).ok()?;
    let models: Vec<(String, String)> = discovered
        .config
        .roles
        .iter()
        .filter_map(|role| {
            let hint =
                kit_model_to_hint(role.model_family.as_deref()?, role.model_size.as_deref())?;
            Some((role.role_id.clone(), hint.to_string()))
        })
        .collect();
    if models.is_empty() {
        None
    } else {
        Some(models)
    }
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

            // Read preferred models from the discovered SwarmKit manifest
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match kit_preferred_models(&cwd) {
                Some(models) => {
                    println!("Preferred Models (from SwarmKit manifest):");
                    for (role, model) in &models {
                        println!("\n  {role}:");
                        println!("    • {model}");
                    }
                    println!();
                }
                None => {
                    // Fallback: check for Gemini
                    println!("Preferred Models:");
                    if std::env::var("GEMINI_API_KEY").is_ok() {
                        println!("  ✓ Gemini API configured");
                    } else {
                        println!("  ✗ No API keys configured");
                        println!("  Set GEMINI_API_KEY to use Gemini models");
                    }
                    println!("\n  Configure preferred models with: arkavo kit init");
                    println!();
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_kit_yaml(role_id: &str, family: &str, size: &str) -> String {
        format!(
            r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "hello"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "say hello"
roles:
  - id: {role_id}
    role_type: operator
    agent_provisioning:
      model:
        family: {family}
        size: {size}
    skills: []
    mcp_tools: []
    handoffs: []
coordination:
  topology: hub-spoke
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  data_classifications: ["public"]
  network:
    egress_allowed: false
    egress_allowlist: []
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures:
    - signer_did: "did:web:example.com"
      algorithm: ed25519
      signature: "AAA"
"#
        )
    }

    #[test]
    fn kit_preferred_models_maps_known_local_edge_model() {
        let dir = tempfile::tempdir().unwrap();
        let arkavo_dir = dir.path().join(".arkavo");
        std::fs::create_dir_all(&arkavo_dir).unwrap();
        std::fs::write(
            arkavo_dir.join("agent.swarmkit.yaml"),
            minimal_kit_yaml("worker", "ministral", "3B"),
        )
        .unwrap();

        let models = kit_preferred_models(dir.path()).expect("kit with known model");
        assert_eq!(
            models,
            vec![("worker".to_string(), "ministral-3b".to_string())]
        );
    }

    #[test]
    fn kit_preferred_models_none_when_no_kit() {
        let dir = tempfile::tempdir().unwrap();
        assert!(kit_preferred_models(dir.path()).is_none());
    }

    #[test]
    fn kit_preferred_models_none_when_model_unrecognized() {
        let dir = tempfile::tempdir().unwrap();
        let arkavo_dir = dir.path().join(".arkavo");
        std::fs::create_dir_all(&arkavo_dir).unwrap();
        std::fs::write(
            arkavo_dir.join("agent.swarmkit.yaml"),
            minimal_kit_yaml("worker", "unknown-family", "1B"),
        )
        .unwrap();

        assert!(kit_preferred_models(dir.path()).is_none());
    }
}
