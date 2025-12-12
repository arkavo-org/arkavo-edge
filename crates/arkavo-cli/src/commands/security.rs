//! CLI commands for data security management.
//!
//! Provides commands to start/stop/status the local OpenTDF stack.

use anyhow::{Context, Result};
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum SecurityCommand {
    /// Start the local OpenTDF data security stack
    Start {
        /// Force restart even if already running
        #[arg(long, short)]
        force: bool,
    },

    /// Stop the local OpenTDF data security stack
    Stop,

    /// Show status of the local OpenTDF stack
    Status,
}

/// Handle security subcommands.
pub async fn handle_security_command(command: SecurityCommand) -> Result<()> {
    // Import the OpenTDF local crate
    use arkavo_opentdf_local::{OpenTdfStack, StackStatus};

    match command {
        SecurityCommand::Start { force } => {
            println!("Starting local OpenTDF data security stack...");

            // Check if stack is already running
            if let Some(stack) = OpenTdfStack::detect().await {
                if !force {
                    let endpoints = stack
                        .get_endpoints()
                        .await
                        .context("Failed to get endpoints")?;

                    println!("Data security already enabled:");
                    println!("  KAS URL: {}", endpoints.kas_url);
                    println!("  OAuth URL: {}", endpoints.oauth_url);
                    println!("  Client ID: {}", endpoints.client_id);
                    println!();
                    println!("Use --force to restart the stack.");
                    return Ok(());
                }

                println!("Force restart requested, stopping existing stack...");
                if let Err(e) = stack.stop().await {
                    eprintln!("Warning: Failed to stop existing stack: {e}");
                }
            }

            // Start the stack
            let stack = OpenTdfStack::new().context("Failed to initialize stack")?;

            stack
                .start()
                .await
                .context("Failed to start OpenTDF stack")?;

            let endpoints = stack
                .get_endpoints()
                .await
                .context("Failed to get endpoints")?;

            println!();
            println!("Data security enabled successfully!");
            println!();
            println!("Endpoints:");
            println!("  KAS URL:        {}", endpoints.kas_url);
            println!("  OAuth URL:      {}", endpoints.oauth_url);
            println!("  Token endpoint: {}", endpoints.token_endpoint);
            println!("  Client ID:      {}", endpoints.client_id);
            println!("  Client Secret:  {}", endpoints.client_secret);
            println!();
            println!("Note: Ensure orchestrator is running on port 3000 for OIDC.");
            println!();
            println!("Usage:");
            println!("  Encrypt: arkavo tdf encrypt --local -i file.txt");
            println!("  Decrypt: arkavo tdf decrypt --local -i file.txt.tdf.json");

            Ok(())
        }

        SecurityCommand::Stop => {
            println!("Stopping local OpenTDF data security stack...");

            match OpenTdfStack::detect().await {
                Some(stack) => {
                    stack.stop().await.context("Failed to stop stack")?;
                    println!("Data security stack stopped.");
                }
                None => {
                    println!("Data security stack is not running.");
                }
            }

            Ok(())
        }

        SecurityCommand::Status => {
            match OpenTdfStack::detect().await {
                Some(stack) => {
                    let status = stack.status().await.context("Failed to get status")?;

                    let status_str = match status {
                        StackStatus::Running => "Running",
                        StackStatus::Partial => "Partially running",
                        StackStatus::Stopped => "Stopped",
                        StackStatus::Unknown => "Unknown",
                    };

                    println!("Data security status: {status_str}");

                    if status == StackStatus::Running {
                        let endpoints = stack
                            .get_endpoints()
                            .await
                            .context("Failed to get endpoints")?;

                        println!();
                        println!("Endpoints:");
                        println!("  KAS URL:        {}", endpoints.kas_url);
                        println!("  OAuth URL:      {}", endpoints.oauth_url);
                        println!("  Token endpoint: {}", endpoints.token_endpoint);
                        println!("  Client ID:      {}", endpoints.client_id);
                    }
                }
                None => {
                    // Try to create stack to check runtime availability
                    match OpenTdfStack::new() {
                        Ok(_) => {
                            println!("Data security status: Not running");
                            println!();
                            println!("Container runtime: Available");
                            println!("Use 'arkavo security start' to enable data security.");
                        }
                        Err(e) => {
                            println!("Data security status: Unavailable");
                            println!();
                            println!("Error: {e}");
                            println!();
                            println!(
                                "Install Docker, Podman, or Apple Container CLI to enable data security."
                            );
                        }
                    }
                }
            }

            Ok(())
        }
    }
}
