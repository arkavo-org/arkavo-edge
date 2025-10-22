//! mDNS Browser for A2A Protocol
//!
//! This utility discovers A2A agents on the local network using mDNS.
//!
//! Usage:
//!   cargo run --example mdns_browser --features mdns
//!   cargo run --example mdns_browser --features mdns -- --register

#![cfg(feature = "mdns")]

use arkavo_protocol::discovery::{DiscoveryConfig, DiscoveryMethod, DiscoveryService};
use arkavo_protocol::mdns::MdnsServiceInfo;
use std::time::Duration;
use tokio::time;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Starting A2A mDNS Browser...");

    // Check command line arguments
    let args: Vec<String> = std::env::args().collect();
    let register_self = args.contains(&"--register".to_string());

    // Create discovery service
    let discovery = DiscoveryService::new(DiscoveryConfig {
        method: DiscoveryMethod::Mdns,
        ..Default::default()
    });

    // Start mDNS discovery
    discovery.start_mdns_discovery().await?;
    info!("Started mDNS discovery, listening for A2A agents...");
    if register_self {
        let info = MdnsServiceInfo {
            agent_id: "mdns-browser".to_string(),
            http_port: 8765,
            ws_port: None,
            version: env!("CARGO_PKG_VERSION").to_string(),
            capabilities: vec!["discovery".to_string()],
        };

        discovery.register_mdns_service(info).await?;
        info!("Registered mDNS browser as discoverable agent");
    }

    // Main discovery loop
    let mut last_count = 0;
    loop {
        // Discover all agents
        match discovery.discover_all() {
            Ok(agents) => {
                if agents.len() != last_count {
                    info!("\n=== Discovered A2A Agents ===");
                    for (i, agent) in agents.iter().enumerate() {
                        info!("{}. Agent ID: {}", i + 1, agent.agent_id);
                        info!("   URL: {}", agent.url);
                    }
                    info!("Total agents: {}\n", agents.len());
                    last_count = agents.len();
                }
            }
            Err(e) => {
                eprintln!("Discovery error: {}", e);
            }
        }

        // Wait before next scan
        time::sleep(Duration::from_secs(5)).await;

        // Check for exit
        if tokio::signal::ctrl_c().await.is_ok() {
            info!("\nShutting down mDNS browser...");
            break;
        }
    }

    // Cleanup
    discovery.shutdown_mdns().await?;
    info!("mDNS browser stopped");

    Ok(())
}

#[cfg(not(feature = "mdns"))]
fn main() {
    eprintln!("This example requires the 'mdns' feature.");
    eprintln!("Run with: cargo run --example mdns_browser --features mdns");
    std::process::exit(1);
}
