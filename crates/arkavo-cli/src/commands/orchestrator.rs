use anyhow::{Result, anyhow};
use arkavo_budget::{BudgetTracker, config::BudgetConfig};
use arkavo_events::{EventWriter, writer::EventWriterConfig};
use arkavo_orchestrator::{
    GitHubApp, GitHubOperations, Orchestrator, OrchestratorConfig, WebhookServer,
    types::IssueAction,
};
use arkavo_protocol::{
    agent_registry::AgentRegistry,
    mcp::McpClient,
    rate_limit::RateLimitConfig,
    task_executor::{TaskExecutor, TaskExecutorConfig},
    task_store::SqliteTaskStore,
};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::signal;
use tracing::{error, info, warn};

const MAX_IP_ENTRIES: usize = 10_000;
const IP_ENTRY_TTL_SECONDS: u64 = 3600;

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
        OrchestratorSubcommand::Process { repo, issue, token } => {
            process_issue(repo.clone(), *issue, token.clone()).await
        }
        OrchestratorSubcommand::Config => show_config(),
        OrchestratorSubcommand::Status => show_status(),
    }
}

async fn start_orchestrator(
    port: u16,
    webhook_secret: Option<String>,
    app_id: Option<String>,
    private_key: Option<String>,
) -> Result<()> {
    info!("Starting GitHub orchestrator server");

    let config = if let (Some(secret), Some(id), Some(key)) = (webhook_secret, app_id, private_key)
    {
        OrchestratorConfig {
            webhook_secret: secret,
            github_app_id: id,
            github_app_private_key: key,
            webhook_port: port,
            ..Default::default()
        }
    } else {
        OrchestratorConfig::from_env()?
    };

    info!("Webhook server will listen on port {}", config.webhook_port);
    info!("GitHub App ID: {}", config.github_app_id);
    info!("Webhook secret: {}", config.get_masked_secret());

    let rate_limit_config = RateLimitConfig {
        max_requests_per_second: config.rate_limit_requests_per_second,
        burst_size: config.rate_limit_burst_size,
        enabled: true,
        max_ip_entries: MAX_IP_ENTRIES,
        ip_entry_ttl_seconds: IP_ENTRY_TTL_SECONDS,
    };

    let (webhook_server, mut event_rx) =
        WebhookServer::new(config.webhook_secret.clone(), rate_limit_config);

    let github_app = Arc::new(GitHubApp::new(
        config.github_app_id.parse()?,
        &config.github_app_private_key,
    )?);

    let github_ops = Arc::new(GitHubOperations::new(Arc::clone(&github_app)));

    let session_id = uuid::Uuid::new_v4().to_string();
    let event_writer = Arc::new(EventWriter::new(EventWriterConfig::default()));
    let budget_tracker = Arc::new(BudgetTracker::new(BudgetConfig::default()).await?);
    let agent_registry = Arc::new(AgentRegistry::new());

    let task_store_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".arkavo")
        .join("orchestrator-tasks.db");
    let task_store = Arc::new(SqliteTaskStore::new(&task_store_path).await?)
        as Arc<dyn arkavo_protocol::task_store::TaskStore>;
    let task_executor = Arc::new(TaskExecutor::new(task_store, TaskExecutorConfig::default()));

    let mcp_client = Arc::new(McpClient::new());

    let orchestrator = Arc::new(
        Orchestrator::new(
            Arc::clone(&task_executor),
            Arc::clone(&agent_registry),
            Arc::clone(&mcp_client),
            Arc::clone(&budget_tracker),
            Arc::clone(&event_writer),
            Arc::clone(&github_ops),
            session_id,
        )
        .await?,
    );

    orchestrator.initialize().await?;

    let app = webhook_server.router();

    let addr = SocketAddr::from(([0, 0, 0, 0], config.webhook_port));

    info!("Starting webhook server on {}", addr);

    let orchestrator_clone = Arc::clone(&orchestrator);
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            if let arkavo_orchestrator::GitHubEvent::Issues(issue_event) = event {
                info!(
                    issue = issue_event.issue.number,
                    repository = %issue_event.repository.full_name,
                    "Received issue event"
                );

                if let Err(e) = orchestrator_clone.handle_issue_event(*issue_event).await {
                    error!(error = %e, "Failed to handle issue event");
                }
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Webhook server listening on {}", addr);
    info!("Orchestrator is ready to process GitHub events");

    let shutdown_signal = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install CTRL+C signal handler");
        info!("Shutdown signal received");
    };

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal)
    .await?;

    orchestrator.stop().await?;
    info!("Orchestrator stopped");

    Ok(())
}

const MAX_PROCESSED_ISSUES: usize = 1000;

#[derive(Debug, Serialize, Deserialize)]
struct PollState {
    last_poll: DateTime<Utc>,
    processed_issues: HashSet<u64>,
    #[serde(default = "default_max_processed")]
    max_processed_issues: usize,
}

fn default_max_processed() -> usize {
    MAX_PROCESSED_ISSUES
}

impl PollState {
    fn new() -> Self {
        Self {
            last_poll: Utc::now(),
            processed_issues: HashSet::new(),
            max_processed_issues: MAX_PROCESSED_ISSUES,
        }
    }

    fn insert_processed(&mut self, issue_number: u64) {
        self.processed_issues.insert(issue_number);

        if self.processed_issues.len() > self.max_processed_issues {
            let to_remove: Vec<u64> = self
                .processed_issues
                .iter()
                .take(self.processed_issues.len() - self.max_processed_issues)
                .copied()
                .collect();
            for num in to_remove {
                self.processed_issues.remove(&num);
            }
        }
    }

    fn state_file_path(repo: &str) -> PathBuf {
        let repo_safe = repo.replace('/', "_");
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".arkavo")
            .join(format!("orchestrator-poll-{repo_safe}.json"))
    }

    fn load(repo: &str) -> Result<Self> {
        let path = Self::state_file_path(repo);
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(Self::new())
        }
    }

    fn save(&self, repo: &str) -> Result<()> {
        let path = Self::state_file_path(repo);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct GitHubIssue {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    html_url: String,
    created_at: String,
    updated_at: String,
    labels: Vec<GitHubLabel>,
}

#[derive(Debug, Deserialize)]
struct GitHubLabel {
    name: String,
}

async fn poll_github(
    repo: String,
    interval: u64,
    once: bool,
    token: Option<String>,
    labels: Option<String>,
) -> Result<()> {
    let token = token.ok_or_else(|| {
        anyhow!("GitHub token required. Set GITHUB_TOKEN environment variable or use --token")
    })?;

    info!("Starting GitHub polling for repository: {}", repo);
    if once {
        info!("Running in one-shot mode");
    } else {
        info!("Polling every {} seconds", interval);
    }

    let mut state = PollState::load(&repo)?;
    let orchestrator = create_polling_orchestrator(&token).await?;

    loop {
        info!("Polling {} for new issues", repo);

        match fetch_new_issues(&repo, &token, &state.last_poll, labels.as_deref()).await {
            Ok(issues) => {
                info!("Found {} issues to process", issues.len());

                for issue in issues {
                    if state.processed_issues.contains(&issue.number) {
                        continue;
                    }

                    info!("Processing issue #{}: {}", issue.number, issue.title);

                    if let Err(e) = process_github_issue(&orchestrator, &repo, &issue).await {
                        error!("Failed to process issue #{}: {}", issue.number, e);
                    } else {
                        state.insert_processed(issue.number);
                    }
                }

                state.last_poll = Utc::now();
                if let Err(e) = state.save(&repo) {
                    warn!("Failed to save poll state: {}", e);
                }
            }
            Err(e) => {
                error!("Failed to fetch issues: {}", e);
            }
        }

        if once {
            info!("One-shot mode complete");
            break;
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(interval)).await;
    }

    Ok(())
}

async fn process_issue(repo: String, issue_number: u64, token: Option<String>) -> Result<()> {
    let token = token.ok_or_else(|| {
        anyhow!("GitHub token required. Set GITHUB_TOKEN environment variable or use --token")
    })?;

    info!("Processing issue #{} from {}", issue_number, repo);

    let orchestrator = create_polling_orchestrator(&token).await?;
    let issue = fetch_issue(&repo, &token, issue_number).await?;

    info!("Issue #{}: {}", issue.number, issue.title);
    process_github_issue(&orchestrator, &repo, &issue).await?;

    info!("Issue processing complete");
    Ok(())
}

async fn create_polling_orchestrator(token: &str) -> Result<Arc<Orchestrator>> {
    let github_app = Arc::new(GitHubApp::new_from_token(token.to_string())?);
    let github_ops = Arc::new(GitHubOperations::new(Arc::clone(&github_app)));

    let session_id = uuid::Uuid::new_v4().to_string();
    let event_writer = Arc::new(EventWriter::new(EventWriterConfig::default()));
    let budget_tracker = Arc::new(BudgetTracker::new(BudgetConfig::default()).await?);
    let agent_registry = Arc::new(AgentRegistry::new());

    let task_store_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".arkavo")
        .join("orchestrator-tasks.db");
    let task_store = Arc::new(SqliteTaskStore::new(&task_store_path).await?)
        as Arc<dyn arkavo_protocol::task_store::TaskStore>;
    let task_executor = Arc::new(TaskExecutor::new(task_store, TaskExecutorConfig::default()));

    let mcp_client = Arc::new(McpClient::new());

    let orchestrator = Arc::new(
        Orchestrator::new(
            Arc::clone(&task_executor),
            Arc::clone(&agent_registry),
            Arc::clone(&mcp_client),
            Arc::clone(&budget_tracker),
            Arc::clone(&event_writer),
            Arc::clone(&github_ops),
            session_id,
        )
        .await?,
    );

    orchestrator.initialize().await?;
    Ok(orchestrator)
}

async fn fetch_new_issues(
    repo: &str,
    token: &str,
    since: &DateTime<Utc>,
    label_filter: Option<&str>,
) -> Result<Vec<GitHubIssue>> {
    fetch_new_issues_impl(
        repo.to_string(),
        token.to_string(),
        *since,
        label_filter.map(|s| s.to_string()),
    )
    .await
}

fn fetch_new_issues_impl(
    repo: String,
    token: String,
    since: DateTime<Utc>,
    label_filter: Option<String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<GitHubIssue>>> + Send>> {
    Box::pin(async move {
        use reqwest::StatusCode;
        use tokio::time::Duration;

        let (owner, repo_name) = parse_repo(&repo)?;
        let client = reqwest::Client::new();

        let mut url = format!(
            "https://api.github.com/repos/{}/{}/issues?state=open&since={}",
            owner,
            repo_name,
            since.to_rfc3339()
        );

        if let Some(labels) = &label_filter {
            url = format!("{url}&labels={labels}");
        }

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("User-Agent", "arkavo-orchestrator")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;

        match response.status() {
            StatusCode::OK => {
                let issues: Vec<GitHubIssue> = response.json().await?;
                Ok(issues)
            }
            StatusCode::FORBIDDEN => {
                if let Some(reset) = response.headers().get("x-ratelimit-reset")
                    && let Ok(reset_str) = reset.to_str()
                    && let Ok(reset_time) = reset_str.parse::<i64>()
                {
                    let now = Utc::now().timestamp();
                    let wait_seconds = (reset_time - now).max(0) as u64;
                    warn!("Rate limited. Waiting {} seconds", wait_seconds);
                    tokio::time::sleep(Duration::from_secs(wait_seconds + 1)).await;
                    return fetch_new_issues_impl(repo, token, since, label_filter).await;
                }
                Err(anyhow!("GitHub API forbidden: {}", response.text().await?))
            }
            status => Err(anyhow!(
                "GitHub API error: {} - {}",
                status,
                response.text().await?
            )),
        }
    })
}

async fn fetch_issue(repo: &str, token: &str, issue_number: u64) -> Result<GitHubIssue> {
    let (owner, repo_name) = parse_repo(repo)?;
    let client = reqwest::Client::new();

    let url = format!("https://api.github.com/repos/{owner}/{repo_name}/issues/{issue_number}");

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "arkavo-orchestrator")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub API error: {} - {}",
            response.status(),
            response.text().await?
        ));
    }

    Ok(response.json().await?)
}

async fn process_github_issue(
    orchestrator: &Arc<Orchestrator>,
    repo: &str,
    issue: &GitHubIssue,
) -> Result<()> {
    use arkavo_orchestrator::{
        IssueEvent,
        types::{Issue, Label, Repository, User},
    };

    let (owner, repo_name) = parse_repo(repo)?;

    let issue_event = IssueEvent {
        action: IssueAction::Opened,
        issue: Issue {
            id: 0,
            number: issue.number,
            title: issue.title.clone(),
            body: issue.body.clone(),
            state: issue.state.clone(),
            html_url: issue.html_url.clone(),
            user: User {
                id: 0,
                login: "unknown".to_string(),
                user_type: "User".to_string(),
                avatar_url: String::new(),
                html_url: String::new(),
            },
            labels: issue
                .labels
                .iter()
                .map(|l| Label {
                    id: 0,
                    name: l.name.clone(),
                    color: String::new(),
                    description: None,
                })
                .collect(),
            assignees: Vec::new(),
            created_at: issue.created_at.clone(),
            updated_at: issue.updated_at.clone(),
            closed_at: None,
            milestone: None,
        },
        repository: Repository {
            id: 0,
            full_name: repo.to_string(),
            name: repo_name.clone(),
            owner: User {
                id: 0,
                login: owner.clone(),
                user_type: "User".to_string(),
                avatar_url: String::new(),
                html_url: String::new(),
            },
            html_url: String::new(),
            description: None,
            default_branch: "main".to_string(),
            language: None,
        },
        sender: User {
            id: 0,
            login: "polling".to_string(),
            user_type: "User".to_string(),
            avatar_url: String::new(),
            html_url: String::new(),
        },
        assignee: None,
    };

    orchestrator.handle_issue_event(issue_event).await?;
    Ok(())
}

fn parse_repo(repo: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        return Err(anyhow!(
            "Invalid repository format. Expected 'owner/repo', got '{repo}'"
        ));
    }

    let owner = parts[0].trim();
    let repo_name = parts[1].trim();

    if owner.is_empty() || repo_name.is_empty() {
        return Err(anyhow!("Owner and repository name cannot be empty"));
    }

    Ok((owner.to_string(), repo_name.to_string()))
}

fn show_config() -> Result<()> {
    let config = OrchestratorConfig::from_env()?;

    println!("Orchestrator Configuration:");
    println!("  GitHub App ID: {}", config.github_app_id);
    println!("  Webhook Secret: {}", config.get_masked_secret());
    println!("  Private Key: {}", config.get_masked_private_key());
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
    println!("  Implementation: Phase 1-2 Complete");
    println!("  Features:");
    println!("    ✓ Webhook server with HMAC-SHA256 verification");
    println!("    ✓ GitHub App authentication with JWT");
    println!("    ✓ Issue analysis and classification");
    println!("    ✓ Intelligent routing (4 strategies)");
    println!("    ✓ Agent assignment and registry");
    println!("    ✓ Cognitive engine for task execution");
    println!("    ✓ Event tracking and metrics");
    println!("  Ready for production use");

    Ok(())
}
