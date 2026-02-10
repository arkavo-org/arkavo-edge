#![allow(clippy::disallowed_methods)] // False positive in clippy

mod capabilities;
mod demo;
mod schema;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cargo xtask")]
#[command(about = "Development task runner for Arkavo Edge")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Run A2A transport demo with two communicating agents")]
    RunDemo {
        #[arg(long, default_value = "8001")]
        agent1_port: u16,
        #[arg(long, default_value = "8002")]
        agent2_port: u16,
        #[arg(long)]
        use_websocket: bool,
    },
    #[command(about = "Generate and validate protocol schemas")]
    SchemaGen {
        #[arg(long, help = "Check if schemas are up to date without modifying files")]
        check: bool,
        #[arg(long, help = "Generate config schemas")]
        config: bool,
        #[arg(long, help = "Generate wire protocol schemas")]
        wire: bool,
    },
    #[command(about = "Browse platform capabilities from specs")]
    Capabilities {
        #[arg(long, help = "Show full capability table")]
        matrix: bool,
        #[arg(long, help = "Compact list with descriptions")]
        list: bool,
        #[arg(long, help = "Filter by name, description, or module")]
        search: Option<String>,
        #[arg(help = "Show detail for a specific capability")]
        name: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::RunDemo {
            agent1_port,
            agent2_port,
            use_websocket,
        } => {
            demo::run_a2a_demo(agent1_port, agent2_port, use_websocket).await?;
        }
        Commands::SchemaGen {
            check,
            config,
            wire,
        } => {
            schema::generate_schemas(check, config, wire)?;
        }
        Commands::Capabilities {
            matrix,
            list,
            search,
            name,
        } => {
            capabilities::run(matrix, list, search, name)?;
        }
    }

    Ok(())
}
