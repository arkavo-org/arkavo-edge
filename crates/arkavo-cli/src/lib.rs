pub mod builtin_mcp;
pub mod commands;
pub mod conversation_manager;
pub mod log;
pub mod mcp_client;
pub mod mcp_integration;
pub mod mcp_spawner;
#[cfg(all(unix, feature = "test-harness"))]
pub mod memory_integration;
pub mod prompt_loader;

#[allow(clippy::disallowed_methods)]
pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
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
        #[cfg(all(target_os = "macos", feature = "test-harness"))]
        "test" => commands::test::execute(&args[1..]),
        #[cfg(not(all(target_os = "macos", feature = "test-harness")))]
        "test" => {
            eprintln!("Test command is not available on this platform");
            Err("Test command requires macOS with test-harness feature (uses iOS simulator)".into())
        }
        // Hidden commands with async runtime
        "model" => {
            let run_async = async {
                use clap::Parser;

                #[derive(Parser)]
                #[command(name = "model")]
                #[command(about = "Manage local LLM models")]
                struct Cli {
                    #[command(flatten)]
                    command: commands::model::ModelCommand,
                }

                let cli = Cli::parse_from(
                    std::iter::once("model")
                        .chain(args[1..].iter().map(std::string::String::as_str)),
                );
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
        #[cfg(all(target_os = "macos", feature = "test-harness"))]
        "serve" | "mcp" => {
            // Always create a new runtime for the MCP server
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async { commands::mcp::run().await })
        }
        #[cfg(not(all(target_os = "macos", feature = "test-harness")))]
        "serve" | "mcp" => {
            eprintln!("MCP server is not available on this platform");
            Err("MCP server requires macOS with test-harness feature (uses iOS simulator)".into())
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
    println!("    chat        Conversational chat");
    println!("    task        Plan and apply code changes");
    println!("    ui          Launch web UI");
    println!("    serve       Run as MCP server");
    println!();
    println!("Run 'arkavo <command> --help' for detailed options");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help       Show help");
    println!("    -v, --version    Show version");
}
