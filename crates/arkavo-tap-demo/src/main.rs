//! `arkavo-tap-demo`: the local Trusted Agent Protocol harness.
//!
//! Development only. The registry is a local trust authority, the grant is a
//! `local-test-grant`, and payment is simulated. Nothing here is a Visa registration, an AP2
//! mandate or a payment.

use anyhow::Context;
use arkavo_tap::{Clock as _, SignedEnvelope, SystemClock};
use arkavo_tap_demo::buyer::{self, Buyer};
use arkavo_tap_demo::config::DemoConfig;
use arkavo_tap_demo::server::{self, SharedClock};
use arkavo_tap_demo::{init, registry, suite};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    version,
    about = "Local Trusted Agent Protocol harness (development only)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create development identities, configuration and a test grant. Never overwrites.
    Init {
        #[arg(long, default_value = ".tap-demo")]
        dir: PathBuf,
        #[arg(long, default_value_t = 3001)]
        merchant_port: u16,
        #[arg(long, default_value_t = 9002)]
        registry_port: u16,
    },
    /// Run the development registry.
    Registry {
        #[arg(long)]
        config: PathBuf,
    },
    /// Run the merchant fixture with TAP verification.
    Merchant {
        #[arg(long)]
        config: PathBuf,
    },
    /// Register the test agents' public keys with the registry.
    Register {
        #[arg(long)]
        config: PathBuf,
    },
    /// Browse a product as the configured agent.
    Browse {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        product: String,
    },
    /// Check out and place a simulated order under a signed test grant.
    Checkout {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        grant: PathBuf,
        #[arg(long, default_value = "demo-001")]
        product: String,
        #[arg(long, default_value_t = 1)]
        quantity: u32,
    },
    /// Show an order placed by the configured agent.
    Order {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        id: String,
    },
    /// Run the acceptance scenarios in an isolated temporary environment.
    VerifySuite,
}

#[tokio::main]
#[allow(clippy::disallowed_methods)] // #[tokio::main] expands to Runtime::block_on
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,arkavo_tap::audit=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let clock: SharedClock = Arc::new(SystemClock);
    match Cli::parse().command {
        Command::Init {
            dir,
            merchant_port,
            registry_port,
        } => {
            init::initialize(&dir, merchant_port, registry_port, clock.now())?;
            println!(
                "Initialized {} (development only: local trust authority, local-test-grant, simulated payment).",
                dir.display()
            );
        }
        Command::Registry { config } => {
            let config = DemoConfig::load(&config)?;
            let registry = server::registry(&config, clock).await?;
            let listener = tokio::net::TcpListener::bind(&config.trust.registry_listen).await?;
            println!(
                "Development registry on {} (a local trust authority, not a Visa registry)",
                config.trust.registry_origin
            );
            let interrupted = async {
                let _ = tokio::signal::ctrl_c().await;
            };
            registry::serve(listener, registry, interrupted).await?;
        }
        Command::Merchant { config } => {
            let config = DemoConfig::load(&config)?;
            let (_merchant, app) = server::merchant(&config, clock).await?;
            let listener = tokio::net::TcpListener::bind(&config.merchant.listen).await?;
            println!(
                "Merchant fixture on {} (simulated payment)",
                config.merchant.external_origin
            );
            axum::serve(listener, app)
                .await
                .context("serving the merchant")?;
        }
        Command::Register { config } => {
            let config = DemoConfig::load(&config)?;
            let k = &config.keys;
            for (path, id) in [
                (&k.agent, &k.agent_key_id),
                (&k.other_agent, &k.other_agent_key_id),
            ] {
                let buyer = Buyer::from_config(&config, path, id, clock.clone())?;
                suite::register(&config, &buyer, None).await?;
                println!("Registered {id} with the development registry");
            }
        }
        Command::Browse { config, product } => {
            let buyer = agent(&config, clock)?;
            show(&buyer.browse(&product).await?);
        }
        Command::Checkout {
            config,
            grant,
            product,
            quantity,
        } => {
            let buyer = agent(&config, clock)?;
            let grant: SignedEnvelope = serde_json::from_str(&std::fs::read_to_string(&grant)?)
                .with_context(|| format!("reading {}", grant.display()))?;
            let key = uuid::Uuid::new_v4().to_string();
            let checkout = buyer
                .checkout(&[(&product, quantity)], &format!("{key}-checkout"))
                .await?;
            show(&checkout);
            let id = checkout.body["checkout"]["checkoutId"]
                .as_str()
                .context("no checkout was created")?;
            show(&buyer.order(id, Some(&grant), &key).await?);
        }
        Command::Order { config, id } => {
            let buyer = agent(&config, clock)?;
            show(&buyer.order_status(&id).await?);
        }
        Command::VerifySuite => {
            let lines = suite::run().await?;
            if !suite::print(&lines) {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

fn agent(config: &std::path::Path, clock: SharedClock) -> anyhow::Result<Buyer> {
    let config = DemoConfig::load(config)?;
    Buyer::from_config(
        &config,
        &config.keys.agent,
        &config.keys.agent_key_id,
        clock,
    )
}

fn show(answer: &buyer::Answer) {
    println!("{}", answer.status);
    println!(
        "{}",
        serde_json::to_string_pretty(&answer.body).unwrap_or_default()
    );
}
