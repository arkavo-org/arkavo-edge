mod demo;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    let cli = Cli::parse();
    
    match cli.command {
        Commands::RunDemo { agent1_port, agent2_port, use_websocket } => {
            demo::run_a2a_demo(agent1_port, agent2_port, use_websocket).await?;
        }
    }
    
    Ok(())
}