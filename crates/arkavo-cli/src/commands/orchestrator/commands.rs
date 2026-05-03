use anyhow::Result;
use arkavo_orchestrator::OrchestratorConfig;
use clap::{Args, Subcommand};

use super::polling::{poll_github, process_issue};
use super::webhook::start_orchestrator;

#[derive(Args)]
pub struct OrchestratorCommand {
    #[command(subcommand)]
    command: OrchestratorSubcommand,
}

#[derive(Subcommand)]
enum OrchestratorSubcommand {
    /// Start the GitHub orchestrator webhook server
    Start {
        /// Port to listen on for webhooks
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Webhook secret for signature verification
        #[arg(long, env = "ARKAVO_GITHUB_WEBHOOK_SECRET")]
        webhook_secret: Option<String>,

        /// GitHub App ID
        #[arg(long, env = "ARKAVO_GITHUB_APP_ID")]
        app_id: Option<String>,

        /// GitHub App private key path
        #[arg(long, env = "ARKAVO_GITHUB_APP_PRIVATE_KEY")]
        private_key: Option<String>,
    },

    /// Poll GitHub for new issues (no webhook required)
    Poll {
        /// Repository to poll (format: owner/repo)
        #[arg(short, long)]
        repo: String,

        /// Poll interval in seconds
        #[arg(short, long, default_value = "300")]
        interval: u64,

        /// Run once and exit (no continuous polling)
        #[arg(long)]
        once: bool,

        /// GitHub personal access token
        #[arg(long, env = "GITHUB_TOKEN")]
        token: Option<String>,

        /// Labels to filter issues (comma-separated)
        #[arg(long)]
        labels: Option<String>,
    },

    /// Poll entire GitHub organization for new issues
    PollOrg {
        /// Organization(s) to poll (can specify multiple)
        #[arg(short, long, required = true)]
        org: Vec<String>,

        /// Poll interval in seconds
        #[arg(short, long, default_value = "300")]
        interval: u64,

        /// Run once and exit (no continuous polling)
        #[arg(long)]
        once: bool,

        /// GitHub personal access token
        #[arg(long, env = "GITHUB_TOKEN")]
        token: Option<String>,

        /// Labels to filter issues (comma-separated)
        #[arg(long)]
        labels: Option<String>,

        /// Regex pattern to include repositories
        #[arg(long)]
        repo_include: Option<String>,

        /// Regex pattern to exclude repositories
        #[arg(long)]
        repo_exclude: Option<String>,

        /// Include archived repositories
        #[arg(long)]
        include_archived: bool,

        /// Maximum concurrent repositories to poll
        #[arg(long, default_value = "10")]
        max_concurrent: usize,

        /// State database path
        #[arg(long)]
        state_db: Option<String>,
    },

    /// Process a specific GitHub issue
    Process {
        /// Repository (format: owner/repo)
        #[arg(short, long)]
        repo: String,

        /// Issue number to process
        #[arg(short, long)]
        issue: u64,

        /// GitHub personal access token
        #[arg(long, env = "GITHUB_TOKEN")]
        token: Option<String>,
    },

    /// Check orchestrator configuration
    Config,

    /// Show orchestrator status and metrics
    Status,

    /// Apply a SwarmKit manifest to the local mesh.
    ///
    /// Loads the kit, asks the running orchestrator's A2A endpoint to
    /// match each role to a capable agent, ship per-role specialization
    /// bundles, launch the SwarmFlight, and dispatch the first task per
    /// role. Reports the assignment plan as JSON on success.
    ApplyKit {
        /// Path to the SwarmKit manifest (.swarmkit.yaml or .json).
        #[arg(value_name = "MANIFEST")]
        path: std::path::PathBuf,

        /// Orchestrator A2A endpoint URL. Defaults to the local
        /// orchestrator-agent's address.
        #[arg(
            long,
            env = "ARKAVO_ORCHESTRATOR_URL",
            default_value = "http://127.0.0.1:8340"
        )]
        orchestrator_url: String,
    },
}

pub async fn run(cmd: &OrchestratorCommand) -> Result<()> {
    match &cmd.command {
        OrchestratorSubcommand::Start {
            port,
            webhook_secret,
            app_id,
            private_key,
        } => {
            start_orchestrator(
                *port,
                webhook_secret.clone(),
                app_id.clone(),
                private_key.clone(),
            )
            .await
        }
        OrchestratorSubcommand::Poll {
            repo,
            interval,
            once,
            token,
            labels,
        } => {
            poll_github(
                repo.clone(),
                *interval,
                *once,
                token.clone(),
                labels.clone(),
            )
            .await
        }
        OrchestratorSubcommand::PollOrg {
            org,
            interval,
            once,
            token,
            labels,
            repo_include,
            repo_exclude,
            include_archived,
            max_concurrent,
            state_db,
        } => {
            let config = arkavo_github::OrgPollingConfig {
                organizations: org.clone(),
                poll_interval_secs: *interval,
                max_concurrent_repos: *max_concurrent,
                repo_include_pattern: repo_include.clone(),
                repo_exclude_pattern: repo_exclude.clone(),
                include_archived: *include_archived,
                labels_filter: labels.clone(),
                state_db_path: state_db.as_ref().map(|s| s.into()),
                github_token: token.clone(),
                run_once: *once,
            };
            arkavo_github::poll_organization(config)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to poll organization: {e}"))
        }
        OrchestratorSubcommand::Process { repo, issue, token } => {
            process_issue(repo.clone(), *issue, token.clone()).await
        }
        OrchestratorSubcommand::Config => show_config(),
        OrchestratorSubcommand::Status => show_status(),
        OrchestratorSubcommand::ApplyKit {
            path,
            orchestrator_url,
        } => apply_kit_via_orchestrator(path, orchestrator_url).await,
    }
}

/// Send the kit's path to a running orchestrator agent over A2A and
/// print the returned assignment plan. The orchestrator agent owns the
/// full apply pipeline; this CLI is just a thin client that lets a
/// human or script trigger it.
async fn apply_kit_via_orchestrator(path: &std::path::Path, orchestrator_url: &str) -> Result<()> {
    let absolute = std::fs::canonicalize(path)
        .map_err(|e| anyhow::anyhow!("resolve manifest path {}: {}", path.display(), e))?;

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "orchestrator.apply_kit",
        "params": {
            "kit_path": absolute.display().to_string(),
        }
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_mins(2))
        .build()
        .map_err(|e| anyhow::anyhow!("build HTTP client: {e}"))?;

    let response = client
        .post(orchestrator_url)
        .json(&request)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("POST {orchestrator_url}: {e}"))?;

    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("parse JSON-RPC response: {e}"))?;

    if !status.is_success() {
        anyhow::bail!("orchestrator returned HTTP {status}: {body}");
    }
    if let Some(err) = body.get("error") {
        anyhow::bail!("orchestrator returned JSON-RPC error: {err}");
    }

    let result = body
        .get("result")
        .ok_or_else(|| anyhow::anyhow!("response missing result field: {body}"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string())
    );
    Ok(())
}

fn show_config() -> Result<()> {
    let config = OrchestratorConfig::from_env()?;

    println!("Orchestrator Configuration:");
    println!("  GitHub App ID: {}", config.github_app_id);
    println!("  Webhook Secret: [set]");
    println!("  Private Key: [set]");
    println!("  Webhook Port: {}", config.webhook_port);
    println!("  Metrics Port: {}", config.metrics_port);
    println!(
        "  Rate Limit: {} req/s (burst: {})",
        config.rate_limit_requests_per_second, config.rate_limit_burst_size
    );
    println!(
        "  Max Request Body: {} MB",
        config.max_request_body_size / (1024 * 1024)
    );

    Ok(())
}

fn show_status() -> Result<()> {
    println!("Orchestrator Status:");
    println!("  Implementation: Phase 1-3 Complete");
    println!("  Features:");
    println!("    ✓ Webhook server with HMAC-SHA256 verification");
    println!("    ✓ GitHub App authentication with JWT");
    println!("    ✓ Personal access token support");
    println!("    ✓ Polling mode (no infrastructure required)");
    println!("    ✓ Issue analysis and classification");
    println!("    ✓ Intelligent routing (4 strategies)");
    println!("    ✓ Agent assignment and registry");
    println!("    ✓ Cognitive engine for task execution");
    println!("    ✓ Event tracking and metrics");
    println!("  Ready for production use");

    Ok(())
}
