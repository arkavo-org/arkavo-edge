#![allow(clippy::collapsible_if)]

use crate::commands::kit::kit_model_to_hint;
use anyhow::Result;
use clap::{Args, Subcommand};
use std::path::{Path, PathBuf};

use super::model_list::{get_model_compatibility, list_local_gguf_models};

#[derive(Args)]
pub struct ModelCommand {
    #[command(subcommand)]
    command: ModelSubcommand,
}

#[derive(Subcommand)]
enum ModelSubcommand {
    /// List available models
    List,

    /// Accepted only so an old invocation gets an explanation instead of a
    /// generic parse error; there is no persistent model selection to switch.
    #[command(hide = true)]
    Switch {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        args: Vec<String>,
    },

    /// Download a model from the registry
    Download {
        /// Name of the model to download (defaults to the model recommended
        /// for this device)
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

        /// Delete the plaintext source after a successful wrap.
        ///
        /// The written archive is reopened, unlocked with the freshly
        /// generated payload key, and its header authenticated first; the
        /// KAS rewrap itself is not exercised, so a wrong KAS public key is
        /// only caught at first load. Keep a backup until a load through
        /// `arkavo login` has succeeded.
        #[arg(long)]
        delete_source: bool,
    },

    /// Accepted only so an old invocation gets an explanation instead of a
    /// generic parse error; models are not registered by hand.
    #[command(hide = true)]
    Add {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        args: Vec<String>,
    },
}

/// Shown when no kit declares preferred models. `kit init` refuses to run
/// without a name, so the hint has to carry the placeholder.
const KIT_INIT_HINT: &str = "Configure preferred models with: arkavo kit init <name>";

const SWITCH_UNSUPPORTED: &str = "'arkavo model switch' is not supported: there is no persistent model selection. Choose a model per run with --model, e.g. 'arkavo chat --model ministral-3b'";

const ADD_UNSUPPORTED: &str = "'arkavo model add' is not supported: models are not registered by hand. Use a local file directly with 'arkavo chat --gguf <path>', or fetch a catalog model with 'arkavo model download <name>'";

const DOWNLOADABLE_MODELS: &str = "Available models:
  gemma-4-e2b   - Gemma 4 E2B (~2.9 GB) - Default small, fast routing
  gemma-4-12b   - Gemma 4 12B (~6.9 GB) - Default medium, most capable
  gemma-4-e4b   - Gemma 4 E4B (~5 GB) - Edge medium
  qwen3.5-0.8b  - Qwen3.5 0.8B (~550 MB) - Best for embedded
  ministral-3b  - Ministral 3B (~2.5 GB)
  ministral-8b  - Ministral 8B (~5.5 GB)
  glm-4.7-flash - GLM-4.7-Flash (~18 GB) - 30B MoE, requires 32GB+ RAM";

/// Map a `model download` name to the model it fetches; no name means the
/// device's recommended model.
///
/// An unrecognized name is an error, not a listing: a script that misspells
/// a model must not see exit 0 and carry on without the weights.
fn resolve_download_model(
    name: Option<&str>,
    recommended: crate::first_run::RecommendedModel,
) -> Result<crate::first_run::RecommendedModel> {
    use crate::first_run::RecommendedModel;

    Ok(match name {
        None => recommended,
        Some("gemma-4-e2b" | "gemma4-e2b" | "gemma-e2b") => RecommendedModel::Gemma4E2B,
        Some("gemma-4-e4b" | "gemma4-e4b" | "gemma-e4b") => RecommendedModel::Gemma4E4B,
        Some("gemma-4-12b" | "gemma4-12b" | "gemma-12b" | "gemma") => RecommendedModel::Gemma4_12B,
        Some("qwen3.5-0.8b" | "qwen3-0.6b" | "qwen" | "qwen3") => RecommendedModel::Qwen35_0_8B,
        Some("ministral-3b" | "ministral3b" | "ministral") => RecommendedModel::Ministral3B,
        Some("ministral-8b" | "ministral8b") => RecommendedModel::Ministral8B,
        Some("glm-4.7-flash" | "glm" | "glm4") => RecommendedModel::Glm47Flash,
        Some(other) => {
            anyhow::bail!("unknown model '{other}'\n\n{DOWNLOADABLE_MODELS}")
        }
    })
}

/// Preferred models declared by the discovered SwarmKit kit: one `(role id,
/// model hint)` pair per role whose `agent_provisioning.model` names a
/// model the router knows, local or cloud, via [`kit_model_to_hint`].
/// Returns `None` when no kit is discovered, or when a kit exists but
/// declares no recognized models — the caller falls back to the same
/// env-key status display either way, matching how the old AGENTS.md-based
/// lookup handled "nothing configured".
fn kit_preferred_models(cwd: &Path) -> Option<Vec<(String, String)>> {
    let discovered = arkavo_swarmkit::load_discovered_kit(cwd).ok()?;
    let models: Vec<(String, String)> = discovered
        .config
        .roles
        .iter()
        .filter_map(|role| {
            let hint =
                kit_model_to_hint(role.model_family.as_deref()?, role.model_size.as_deref())?;
            Some((role.role_id.clone(), hint))
        })
        .collect();
    if models.is_empty() {
        None
    } else {
        Some(models)
    }
}

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
                    println!("\n  {KIT_INIT_HINT}");
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

        ModelSubcommand::Switch { .. } => anyhow::bail!(SWITCH_UNSUPPORTED),

        ModelSubcommand::Download { name } => {
            use crate::first_run::{RecommendedModel, detect_capabilities, download_model};

            let caps = detect_capabilities();

            let model = resolve_download_model(name.as_deref(), caps.recommended_model)?;
            if name.is_none() {
                println!(
                    "No model specified, using recommended: {}",
                    model.display_name()
                );
            }

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

        ModelSubcommand::Add { .. } => anyhow::bail!(ADD_UNSUPPORTED),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

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

    /// `ModelCommand` derives `Args`, not `Parser`, so it has no `Command` of
    /// its own to render help from; this wrapper mirrors the one `lib.rs`
    /// builds at dispatch time.
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        command: ModelCommand,
    }

    fn help_for(subcommand: &str) -> String {
        let mut top = Cli::command();
        top.find_subcommand_mut(subcommand)
            .unwrap_or_else(|| panic!("{subcommand} subcommand exists"))
            .render_long_help()
            .to_string()
    }

    fn advertised_subcommands() -> Vec<String> {
        Cli::command()
            .get_subcommands()
            .filter(|sub| !sub.is_hide_set())
            .map(|sub| sub.get_name().to_string())
            .collect()
    }

    fn parse(args: &[&str]) -> ModelCommand {
        Cli::try_parse_from(std::iter::once("model").chain(args.iter().copied()))
            .unwrap_or_else(|e| panic!("{args:?} should parse: {e}"))
            .command
    }

    /// Regression: the help named `gemma3-1b-it-qat` as the default while
    /// the code downloads the device-recommended Gemma 4 model.
    #[test]
    fn download_help_describes_the_default_the_code_uses() {
        let help = help_for("download");
        assert!(!help.contains("gemma3"), "stale default in:\n{help}");
        assert!(
            help.contains("recommended"),
            "help should name the device-recommended default, got:\n{help}"
        );
    }

    #[test]
    fn download_without_a_name_uses_the_recommended_model() {
        use crate::first_run::RecommendedModel;
        for recommended in [RecommendedModel::Gemma4_12B, RecommendedModel::Gemma4E4B] {
            assert_eq!(
                resolve_download_model(None, recommended).unwrap(),
                recommended
            );
        }
    }

    #[test]
    fn download_resolves_catalog_names_regardless_of_the_recommendation() {
        use crate::first_run::RecommendedModel;
        assert_eq!(
            resolve_download_model(Some("ministral-3b"), RecommendedModel::Gemma4_12B).unwrap(),
            RecommendedModel::Ministral3B
        );
        assert_eq!(
            resolve_download_model(Some("gemma-4-e2b"), RecommendedModel::Gemma4_12B).unwrap(),
            RecommendedModel::Gemma4E2B
        );
    }

    /// Every name the error offers must itself resolve, or the listing
    /// sends the user to another failure.
    #[test]
    fn every_listed_model_name_resolves() {
        use crate::first_run::RecommendedModel;
        let names: Vec<&str> = DOWNLOADABLE_MODELS
            .lines()
            .skip(1)
            .filter_map(|line| line.split_whitespace().next())
            .collect();
        assert_eq!(names.len(), 7, "{names:?}");
        for name in names {
            assert!(
                resolve_download_model(Some(name), RecommendedModel::Gemma4E2B).is_ok(),
                "{name} is listed but does not resolve"
            );
        }
    }

    /// Regression: an unknown model name printed the catalog and exited 0.
    #[tokio::test]
    async fn download_of_an_unknown_model_is_an_error() {
        let err = run(&parse(&["download", "no-such-model"]))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown model 'no-such-model'"), "{err}");
        assert!(
            err.contains("gemma-4-12b"),
            "error should list names: {err}"
        );
    }

    /// Regression: `model switch` was advertised, printed "not yet
    /// implemented" and exited 0.
    #[tokio::test]
    async fn switch_is_not_advertised_and_fails_when_invoked() {
        let advertised = advertised_subcommands();
        assert!(
            !advertised.contains(&"switch".to_string()),
            "{advertised:?}"
        );
        assert!(
            advertised.contains(&"download".to_string()),
            "{advertised:?}"
        );

        let err = run(&parse(&["switch", "ministral-3b"]))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not supported"), "{err}");
        assert!(
            err.contains("--model"),
            "error should say what to do: {err}"
        );
    }

    #[tokio::test]
    async fn add_is_not_advertised_and_fails_when_invoked() {
        let advertised = advertised_subcommands();
        assert!(!advertised.contains(&"add".to_string()), "{advertised:?}");

        let err = run(&parse(&["add", "./model.gguf", "--name", "mine"]))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("not supported"), "{err}");
    }

    /// Regression: the hint read `arkavo kit init`, which fails for want of
    /// the name it requires.
    #[test]
    fn kit_init_hint_includes_the_required_name() {
        assert!(KIT_INIT_HINT.contains("arkavo kit init <name>"));
    }

    #[test]
    fn delete_source_help_states_the_kas_rewrap_is_not_exercised() {
        let mut top = Cli::command();
        let protect = top
            .find_subcommand_mut("protect")
            .expect("protect subcommand exists");
        let help = protect.render_long_help().to_string();
        assert!(
            help.contains("KAS rewrap itself is not exercised"),
            "help must state the trust boundary, got:\n{help}"
        );
    }
}
