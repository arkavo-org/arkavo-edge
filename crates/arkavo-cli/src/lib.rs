pub mod commands;
pub mod first_run;
pub mod hardware;
pub mod mcp_client;
pub mod mcp_integration;
pub mod mcp_spawner;
#[cfg(all(unix, feature = "mcp-tools"))]
pub mod memory_integration;
pub mod mock_llm_server;
pub mod mock_provider;
pub mod prompt_loader;
pub mod secure_http;
pub mod tool_integration;
pub mod welcome;

#[allow(clippy::disallowed_methods)]
pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber for Router quality gate logging
    // Respects RUST_LOG environment variable (e.g., RUST_LOG=debug)
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        use tracing_subscriber::{EnvFilter, fmt};
        fmt()
            .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
            .with_env_filter(
                // Default to error-only for clean CLI output; use RUST_LOG for more
                EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("error")),
            )
            .with_target(false)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .init();

        // Load API keys from .arkavo/AGENTS.md if present
        arkavo_router::model_discovery::load_api_keys_from_config();

        // Initialize security controls
        // SECURITY: Egress filter prevents SSRF attacks
        secure_http::init_egress_filter();
    });

    // Check for verbose flag
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");

    // Skip first-run for help/version commands
    let is_help_or_version = args
        .first()
        .is_some_and(|a| matches!(a.as_str(), "-h" | "--help" | "help" | "-v" | "--version"));

    // First-run experience: check if models are available
    if !is_help_or_version && first_run::is_first_run() {
        // Handle first-run flow in a runtime
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(handle_first_run(verbose))?;
    }

    if args.is_empty() {
        // No command provided, default to agent run
        return commands::agent::execute(&["run".to_string()]);
    }

    match args[0].as_str() {
        "agent" => commands::agent::execute(&args[1..]),
        "chat" => commands::chat::execute(&args[1..]),
        "task" => commands::task::execute(&args[1..]),
        "ui" => commands::ui::execute(&args[1..]),
        // Hidden commands (still accessible, just not in main help)
        "terminal" => commands::terminal::execute(&args[1..]),
        #[cfg(all(target_os = "macos", feature = "mcp-macos"))]
        "test" => commands::test::execute(&args[1..]),
        #[cfg(not(all(target_os = "macos", feature = "mcp-macos")))]
        "test" => {
            eprintln!("Test command is not available on this platform");
            Err("Test command requires macOS with mcp-tools feature (uses iOS simulator)".into())
        }
        // Hidden commands with async runtime
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
        #[cfg(feature = "llama-cpp")]
        "tool-bench" => {
            let run_async = async {
                use clap::Parser;

                #[derive(Parser)]
                #[command(name = "tool-bench")]
                #[command(about = "Benchmark tool calling across local models")]
                struct Cli {
                    #[command(flatten)]
                    command: commands::tool_bench::ToolBenchCommand,
                }

                let cli = Cli::parse_from(
                    std::iter::once("tool-bench")
                        .chain(args[1..].iter().map(std::string::String::as_str)),
                );
                commands::tool_bench::run(&cli.command).await
            };

            match tokio::runtime::Handle::try_current() {
                Ok(handle) => handle.block_on(run_async),
                Err(_) => {
                    let runtime = tokio::runtime::Runtime::new()?;
                    runtime.block_on(run_async)
                }
            }
        }
        #[cfg(feature = "eval-tool")]
        "eval" => {
            let run_async = async {
                match args.get(1).map(|s| s.as_str()) {
                    Some("run") => commands::eval::run(&args[2..]).await,
                    _ => Err(
                        "usage: arkavo eval run --contract <path> [--answer id=text] [--main]"
                            .into(),
                    ),
                }
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
        _ => {
            eprintln!("Error: Unknown command '{}'", args[0]);
            print_usage();
            Err(format!("Unknown command: {}", args[0]).into())
        }
    }
}

fn print_usage() {
    println!("Arkavo Edge");
    println!();
    println!("USAGE:");
    println!("    arkavo [COMMAND] [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("    chat           Conversational chat");
    println!("    task           Plan and apply code changes");
    println!("    ui             Launch web UI");
    println!();
    println!("Run 'arkavo <command> --help' for detailed options");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help       Show help");
    println!("    -v, --version    Show version");
    println!("    --trust          Run the agent and show its authorization QR code (DID:key)");
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
        println!("Or use a cloud provider with an API key:");
        println!("  GEMINI_API_KEY=your-key arkavo chat --prompt \"Hello\"");
    }

    std::process::exit(0);
}
