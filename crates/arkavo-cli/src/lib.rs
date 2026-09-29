pub mod cloud_consent;
pub mod commands;
pub mod first_run;
pub mod hardware;
pub mod logging;
pub mod mcp_client;
pub mod mcp_integration;
pub mod mcp_spawner;
#[cfg(all(unix, feature = "mcp-tools"))]
pub mod memory_integration;
pub mod mock_llm_server;
pub mod mock_provider;
pub mod prompt_loader;
pub mod secure_http;
pub mod security_command;
#[cfg(feature = "sentinel")]
pub mod sentinel_embedder;
#[cfg(feature = "sentinel")]
pub mod sentinel_scorer;
#[cfg(feature = "sentinel")]
pub mod sentinel_wiring;
pub mod startup_policy;
pub mod tool_integration;
pub mod welcome;

#[allow(clippy::disallowed_methods)]
pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        logging::init();

        // Initialize security controls
        // SECURITY: Egress filter prevents SSRF attacks
        secure_http::init_egress_filter();
        #[cfg(feature = "sentinel")]
        sentinel_wiring::install();
    });

    // Check for verbose flag
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");

    // Skip first-run for help/version commands
    let is_help_or_version = args
        .first()
        .is_some_and(|a| matches!(a.as_str(), "-h" | "--help" | "help" | "-v" | "--version"));

    // A command line that will be refused is refused here, before first-run
    // setup can offer a model download on its behalf.
    if args.first().is_some_and(|command| command == "chat") {
        commands::chat::check_args(&args[1..])?;
    }

    startup_policy::validate_local_backend(
        args,
        cfg!(any(feature = "llama-cpp", feature = "snpe")),
    )?;
    if !is_help_or_version && startup_policy::needs_local_setup(args) && first_run::is_first_run() {
        match startup_policy::first_run_action() {
            startup_policy::FirstRunAction::Prompt => {
                let runtime = tokio::runtime::Runtime::new()?;
                runtime.block_on(handle_first_run(verbose))?;
            }
            startup_policy::FirstRunAction::Skip => {
                return Err("The agent harness requires local models. Provision them with `arkavo model download` before starting; cloud credentials do not replace local inference.".into());
            }
        }
    }

    if args.is_empty() {
        // No command provided, default to agent run
        return commands::agent::execute(&["run".to_string()]);
    }

    match args[0].as_str() {
        "agent" => commands::agent::execute(&args[1..]),
        "kit" => commands::kit::execute(&args[1..]),
        "chat" => commands::chat::execute(&args[1..]),
        "task" => commands::task::execute(&args[1..]),
        "ui" => commands::ui::execute(&args[1..]),
        "mcp" => commands::mcp_proxy::execute(&args[1..]),
        command @ ("login" | "logout") => commands::login::execute(command, &args[1..]),
        "security" => security_command::execute(&args[1..]),
        #[cfg(feature = "knowledge-pack")]
        "pack" => commands::pack::execute(&args[1..]).map_err(Into::into),
        #[cfg(not(feature = "knowledge-pack"))]
        "pack" => Err("pack is not in this build; compile with the knowledge-pack feature".into()),
        // Hidden commands (still accessible, just not in main help)
        "terminal" => commands::terminal::execute(&args[1..]),
        #[cfg(all(target_os = "macos", feature = "mcp-macos"))]
        "test" => commands::test::execute(&args[1..]),
        #[cfg(not(all(target_os = "macos", feature = "mcp-macos")))]
        "test" => {
            eprintln!("Test command is not available on this platform");
            Err("Test command requires macOS with mcp-tools feature (uses iOS simulator)".into())
        }
        "model" | "models" | "ls" => {
            let run_async = async {
                use clap::Parser;

                #[derive(Parser)]
                #[command(name = "model")]
                #[command(about = "Manage local LLM models")]
                struct Cli {
                    #[command(flatten)]
                    command: commands::model::ModelCommand,
                }

                // If called as 'ls' with no args, default to 'list' subcommand
                let command_name = args[0].as_str();
                let effective_args = if command_name == "ls" && args.len() == 1 {
                    vec!["model".to_string(), "list".to_string()]
                } else {
                    std::iter::once("model")
                        .chain(args[1..].iter().map(std::string::String::as_str))
                        .map(String::from)
                        .collect()
                };

                let cli = Cli::parse_from(effective_args);
                commands::model::run(&cli.command)
                    .await
                    .map_err(std::convert::Into::into)
            };

            match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle.block_on(run_async),
                Err(_) => {
                    let runtime = tokio::runtime::Runtime::new()?;
                    runtime.block_on(run_async)
                }
            }
        }
        // Hidden, like `terminal` and `test` above
        "dataflow" | "flow" => {
            let run_async = async {
                use clap::Parser;

                #[derive(Parser)]
                #[command(name = "dataflow")]
                #[command(about = "Manage dataflow pipelines")]
                struct Cli {
                    #[command(subcommand)]
                    command: commands::dataflow::DataflowCommand,
                }

                let cli = Cli::parse_from(
                    std::iter::once("dataflow")
                        .chain(args[1..].iter().map(std::string::String::as_str)),
                );
                commands::dataflow::handle_dataflow_command(cli.command)
                    .await
                    .map_err(std::convert::Into::into)
            };

            match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle.block_on(run_async),
                Err(_) => {
                    let runtime = tokio::runtime::Runtime::new()?;
                    runtime.block_on(run_async)
                }
            }
        }
        "help" => {
            print_usage();
            Ok(())
        }
        "-h" | "--help" => {
            print_usage();
            Ok(())
        }
        // Leading options with no subcommand run the default `agent` command, so
        // `arkavo --trust` behaves like `arkavo agent run --trust`. (`-v`/`--version`
        // and `-h`/`--help` are handled above / in main before reaching here.)
        flag if flag.starts_with('-') => commands::agent::execute(args),
        unknown => Err(unknown_command_message(unknown).into()),
    }
}

/// The whole report for a mistyped command: what was wrong and where to
/// look. The caller prints a returned error once, so nothing is printed
/// here; printing the error, the full usage and then the error again buried
/// the one line the user needed.
fn unknown_command_message(command: &str) -> String {
    format!("unknown command '{command}'\nRun 'arkavo --help' for a list of commands")
}

fn print_usage() {
    println!("{}", usage_text());
}

/// The top-level help. Lists what this build can actually run: `pack`
/// appears only when the knowledge-pack feature is compiled in, because a
/// build without it refuses the command.
fn usage_text() -> String {
    let mut commands = vec![
        "    agent          Run an agent (the default when no command is given)",
        "    chat           Conversational chat",
        "    kit            Author and validate SwarmKit manifests",
        "    model          List and download local models",
        "    task           Plan and apply code changes",
        "    ui             Launch web UI",
    ];
    if cfg!(feature = "knowledge-pack") {
        commands.push("    pack           Build sealed knowledge-pack components");
    }
    commands.push("    mcp proxy      Permit-gated stdio MCP relay");
    commands.push(security_command::help_line());
    commands.push(commands::login::login_help());

    format!(
        "Arkavo Edge

USAGE:
    arkavo [COMMAND] [OPTIONS]

COMMANDS:
{}

Run 'arkavo <command> --help' for detailed options

OPTIONS:
    -h, --help       Show help
    -v, --version    Show version
    --trust          Run the agent on loopback only and show its authorization QR code (DID:key)",
        commands.join("\n")
    )
}

/// Handle first-run experience for new users
async fn handle_first_run(verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    use first_run::RecommendedModel;

    let caps = first_run::detect_capabilities();

    // Display verbose welcome with QR code if requested
    if !verbose || welcome::display_welcome_verbose().is_err() {
        println!("Welcome Friend\n");
    }

    // Small model for fast routing, medium model for capable agentic inference.
    let small_model = RecommendedModel::Gemma4E2B;
    let large_model = caps.recommended_model;

    let small_gb = small_model.size_bytes() as f64 / 1_000_000_000.0;
    let large_gb = large_model.size_bytes() as f64 / 1_000_000_000.0;
    let total_gb = small_gb + large_gb;

    println!("Arkavo Edge runs AI locally. First-time setup downloads two models:");
    println!();
    println!(
        "  Small (fast routing):  {} ({:.1} GB)",
        small_model.display_name(),
        small_gb
    );
    println!(
        "  Medium (inference):    {} ({:.1} GB)",
        large_model.display_name(),
        large_gb
    );
    println!();
    println!("  System:      {}", caps.device_profile);
    println!("  Total size:  {total_gb:.1} GB");
    println!("  Disk space:  {:.1} GB available", caps.available_disk_gb);

    // Prompt for download
    if first_run::prompt_download_both(&caps, total_gb) {
        // Download small model first (faster)
        println!();
        match first_run::download_model(&small_model).await {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Download failed: {e}");
                return Err(e.into());
            }
        }

        // Download large model
        println!();
        match first_run::download_model(&large_model).await {
            Ok(_) => {
                println!("\nModels ready! Run 'arkavo' to start.");
            }
            Err(e) => {
                eprintln!("Download failed: {e}");
                return Err(e.into());
            }
        }
    } else {
        println!();
        println!("You can download models later with:");
        println!("  arkavo model download");
        println!();
    }

    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed_commands() -> Vec<String> {
        usage_text()
            .lines()
            .skip_while(|line| *line != "COMMANDS:")
            .skip(1)
            .take_while(|line| !line.is_empty())
            .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
            .collect()
    }

    /// Regression: the help omitted `agent` and `model`, although the README
    /// and the first-run message tell users to run them.
    #[test]
    fn usage_lists_agent_and_model() {
        let listed = listed_commands();
        for command in ["agent", "model"] {
            assert!(
                listed.contains(&command.to_string()),
                "{command} missing from {listed:?}"
            );
        }
    }

    #[test]
    fn usage_says_that_trust_keeps_the_agent_on_loopback() {
        let usage = usage_text();
        let trust = usage
            .lines()
            .find(|line| line.trim_start().starts_with("--trust"))
            .expect("--trust is listed");
        assert!(trust.contains("loopback"), "{trust}");
        assert!(trust.contains("QR code"), "{trust}");
    }

    /// Regression: the help advertised `pack` in builds that reject it.
    #[test]
    fn usage_lists_pack_only_when_it_is_compiled_in() {
        assert_eq!(
            listed_commands().contains(&"pack".to_string()),
            cfg!(feature = "knowledge-pack")
        );
    }

    /// Regression: an unknown command printed an error, the full usage and
    /// then a second, differently worded error.
    #[test]
    fn unknown_command_is_reported_once_with_a_pointer_to_help() {
        let err = run(&["frobnicate".to_string()]).unwrap_err().to_string();
        let lines: Vec<&str> = err.lines().collect();
        assert_eq!(
            lines,
            [
                "unknown command 'frobnicate'",
                "Run 'arkavo --help' for a list of commands"
            ]
        );
    }

    /// Regression: `arkavo security audit` had no dispatch arm and ended in
    /// "unknown command", so the audit could not be run at all.
    #[test]
    fn security_is_dispatched_and_listed() {
        assert!(listed_commands().contains(&"security".to_string()));

        run(&["security".to_string(), "--help".to_string()])
            .expect("security --help is a known command");
        run(&[
            "security".to_string(),
            "audit".to_string(),
            "--help".to_string(),
        ])
        .expect("security audit --help is a known command");

        let err = run(&["security".to_string(), "scan".to_string()])
            .unwrap_err()
            .to_string();
        assert_eq!(
            err.lines().next(),
            Some("unknown security subcommand 'scan'")
        );
    }

    #[test]
    fn usage_still_lists_the_other_commands() {
        let listed = listed_commands();
        for command in ["chat", "kit", "task", "ui", "mcp", "login", "logout"] {
            assert!(
                listed.contains(&command.to_string()),
                "{command} missing from {listed:?}"
            );
        }
    }
}
