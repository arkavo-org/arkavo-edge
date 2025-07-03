pub mod commands;
pub mod conversation_manager;
pub mod mcp_client;
pub mod mcp_integration;
pub mod memory_integration;
pub mod repository_context;

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.is_empty() {
        // No command provided, default to chat
        return commands::chat::execute(&[]);
    }

    match args[0].as_str() {
        "agent" => commands::agent::execute(&args[1..]),
        "chat" => commands::chat::execute(&args[1..]),
        "plan" => commands::plan::execute(&args[1..]),
        "apply" => commands::apply::execute(&args[1..]),
        "test" => commands::test::execute(&args[1..]),
        "ui" => commands::ui::execute(&args[1..]),
        "vault" => commands::vault::execute(&args[1..]),
        "model" => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async {
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
            })
        }
        "dataflow" | "flow" => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async {
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
            })
        }
        "serve" | "mcp" => {
            let runtime = tokio::runtime::Runtime::new()?;
            runtime.block_on(async { commands::mcp::run().await })
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
    println!("    arkavo <COMMAND> [OPTIONS]");
    println!();
    println!("COMMANDS:");
    println!("    agent     Configure and run AI agents");
    println!(
        "    chat      Start conversational agent with repository context (Terminal UI by default)"
    );
    println!(
        "              Options: --no-tui (disable Terminal UI), --prompt <text>, --image <path>"
    );
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
