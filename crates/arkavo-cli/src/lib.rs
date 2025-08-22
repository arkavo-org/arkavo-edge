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
        "terminal" => commands::terminal::execute(&args[1..]),
        "plan" => commands::plan::execute(&args[1..]),
        "apply" => commands::apply::execute(&args[1..]),
        #[cfg(all(target_os = "macos", feature = "test-harness"))]
        "test" => commands::test::execute(&args[1..]),
        #[cfg(not(all(target_os = "macos", feature = "test-harness")))]
        "test" => {
            eprintln!("Test command is not available on this platform");
            Err("Test command requires macOS with test-harness feature (uses iOS simulator)".into())
        }
        "ui" => commands::ui::execute(&args[1..]),
        "vault" => commands::vault::execute(&args[1..]),
        "model" => {
            // Check if we're already in a runtime context
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
    println!("Arkavo Edge - Developer-centric agentic CLI tool for AI-agent development");
    println!();
    println!("USAGE:");
    println!("    arkavo              Run default agent (auto-generates config if needed)");
    println!("    arkavo <COMMAND>    Run specific command");
    println!();
    println!("COMMANDS:");
    println!("    agent     Configure and run AI agents (default when no command given)");
    println!("    chat      Simple conversational chat interface");
    println!("              Options: --prompt <text>, --image <path>, --max-tokens <n>");
    println!("                       --model <name|path> (e.g., tinyllama, gemma-2b, phi-2)");
    println!("                       --model <repo/model> (e.g., TheBloke/phi-2-GGUF)");
    println!("                       --model <path.gguf> (direct path to GGUF file)");
    println!("                       --repo-context {{auto|on|off}} (default: auto)");
    println!("              Use /new in chat to start fresh session");
    println!("              Use /context {{auto|on|off}} to toggle context in REPL");
    println!("    terminal  Launch Terminal UI with streaming chat interface");
    println!("              Options: --model <name> (gemma-3-270m or gemma-2-2b)");
    println!("    plan      Generate a change plan before code edits");
    println!("    apply     Execute plan and commit changes");
    println!("    test      Run intelligent tests (use --help for modes)");
    println!("    ui        Launch web UI for agent orchestration");
    println!("    vault     Import/export notes to Edge Vault");
    println!("    model     Manage local LLM models (list, download, switch, add)");
    println!("    dataflow  Manage dataflow pipelines (export/import blueprints)");
    println!("    serve     Run as MCP server for AI tools integration");
    println!("    help      Print this help message");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help       Print help information");
    println!("    -v, --version    Print version information");
}
