pub mod builtin_mcp;
pub mod commands;
pub mod conversation_manager;
pub mod log;
pub mod mcp_client;
pub mod mcp_integration;
pub mod mcp_polling;
pub mod mcp_spawner;
#[cfg(all(unix, feature = "mcp-tools"))]
pub mod memory_integration;
pub mod peer_manager;
pub mod prompt_loader;
pub mod tool_integration;

#[allow(clippy::disallowed_methods)]
pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber for Router quality gate logging
    // Respects RUST_LOG environment variable (e.g., RUST_LOG=debug)
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        use tracing_subscriber::{EnvFilter, fmt};
        fmt()
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
    });

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
        #[cfg(all(target_os = "macos", feature = "mcp-tools"))]
        "test" => commands::test::execute(&args[1..]),
        #[cfg(not(all(target_os = "macos", feature = "mcp-tools")))]
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
        "orchestrator" => {
            let run_async = async {
                use clap::Parser;

                #[derive(Parser)]
                #[command(name = "orchestrator")]
                #[command(about = "GitHub issue orchestration")]
                struct Cli {
                    #[command(flatten)]
                    command: commands::orchestrator::OrchestratorCommand,
                }

                let cli = Cli::parse_from(
                    std::iter::once("orchestrator")
                        .chain(args[1..].iter().map(std::string::String::as_str)),
                );
                commands::orchestrator::run(&cli.command)
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
        #[cfg(all(target_os = "macos", feature = "mcp-tools"))]
        "serve" | "mcp" => {
            // Always create a new runtime for the MCP server
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async { commands::mcp::run().await })
        }
        #[cfg(not(all(target_os = "macos", feature = "mcp-tools")))]
        "serve" | "mcp" => {
            eprintln!("MCP server is not available on this platform");
            Err("MCP server requires macOS with mcp-tools feature (uses iOS simulator)".into())
        }
        "tdf" => {
            let run_async = async {
                use clap::Parser;

                #[derive(Parser)]
                #[command(name = "tdf")]
                #[command(about = "TDF encryption, decryption, and P2P transport")]
                struct Cli {
                    #[command(subcommand)]
                    command: commands::tdf::TdfCommand,
                }

                let cli = Cli::parse_from(
                    std::iter::once("tdf").chain(args[1..].iter().map(std::string::String::as_str)),
                );
                commands::tdf::handle_tdf_command(cli.command)
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
    println!("    orchestrator   GitHub issue orchestration");
    println!("    serve          Run as MCP server");
    println!("    tdf            TDF encryption and P2P transport");
    println!();
    println!("Run 'arkavo <command> --help' for detailed options");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help       Show help");
    println!("    -v, --version    Show version");
}
