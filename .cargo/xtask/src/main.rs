#![allow(clippy::disallowed_methods)] // False positive in clippy

mod binary_size;
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
    #[command(about = "Build release binary and verify size against the workspace budget")]
    CheckBinarySize {
        #[arg(long, default_value_t = 60, help = "Maximum allowed size in MiB")]
        limit_mb: u64,
        #[arg(long, help = "Package name to build (defaults to arkavo)")]
        package: Option<String>,
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
        Commands::CheckBinarySize { limit_mb, package } => {
            binary_size::check(limit_mb, package)?;
        }
    }

    Ok(())
}
