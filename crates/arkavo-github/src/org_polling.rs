use crate::error::{GitHubError, Result};
use crate::poller::{OrgPoller, OrgPollerConfig};
use arkavo_budget::{BudgetTracker, config::BudgetConfig};
use arkavo_events::{EventWriter, writer::EventWriterConfig};
use arkavo_orchestrator::{GitHubApp, GitHubOperations, Orchestrator};
use arkavo_protocol::{
    agent_registry::AgentRegistry,
    mcp::McpClient,
    task_executor::{TaskExecutor, TaskExecutorConfig},
    task_store::SqliteTaskStore,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

pub struct OrgPollingConfig {
    pub organizations: Vec<String>,
    pub poll_interval_secs: u64,
    pub max_concurrent_repos: usize,
    pub repo_include_pattern: Option<String>,
    pub repo_exclude_pattern: Option<String>,
    pub include_archived: bool,
    pub labels_filter: Option<String>,
    pub state_db_path: Option<PathBuf>,
    pub github_token: Option<String>,
    pub run_once: bool,
}

impl Default for OrgPollingConfig {
    fn default() -> Self {
        Self {
            organizations: Vec::new(),
            poll_interval_secs: 300,
            max_concurrent_repos: 10,
            repo_include_pattern: None,
            repo_exclude_pattern: None,
            include_archived: false,
            labels_filter: None,
            state_db_path: None,
            github_token: None,
            run_once: false,
        }
    }
}

pub async fn poll_organization(config: OrgPollingConfig) -> Result<()> {
    if config.organizations.is_empty() {
        return Err(GitHubError::GitHubApi(
            "At least one organization must be specified".to_string(),
        ));
    }

    let token = config.github_token.ok_or_else(|| {
        GitHubError::GitHubApi(
            "GitHub token required. Set GITHUB_TOKEN environment variable or use --token.\nNote: Unauthenticated access is limited to 60 requests/hour.".to_string(),
        )
    })?;

    info!(
        "Starting organization polling for: {}",
        config.organizations.join(", ")
    );

    let state_db_path = config
        .state_db_path
        .unwrap_or_else(|| PathBuf::from(".arkavo/orchestrator-org-state.db"));

    let orchestrator = create_orchestrator(&token).await?;

    let poller_config = OrgPollerConfig {
        poll_interval: Duration::from_secs(config.poll_interval_secs),
        max_concurrent_repos: config.max_concurrent_repos,
        discovery_interval: Duration::from_secs(300),
        repo_include_pattern: config.repo_include_pattern,
        repo_exclude_pattern: config.repo_exclude_pattern,
        include_archived: config.include_archived,
        labels_filter: config.labels_filter,
    };

    let pollers = {
        let mut pollers = Vec::new();
        for org in &config.organizations {
            let poller = OrgPoller::new(
                org.clone(),
                Some(token.clone()),
                orchestrator.clone(),
                state_db_path.clone(),
                poller_config.clone(),
            )
            .await?;

            poller.start().await?;
            pollers.push(poller);
        }
        pollers
    };

    if config.run_once {
        info!("Running in one-shot mode, waiting for completion...");
        tokio::time::sleep(Duration::from_secs(config.poll_interval_secs + 10)).await;

        for poller in pollers {
            poller.stop().await?;
        }

        info!("One-shot polling complete");
    } else {
        info!(
            "Polling {} organizations continuously. Press Ctrl+C to stop.",
            config.organizations.len()
        );

        tokio::signal::ctrl_c().await?;

        info!("Shutting down pollers...");
        for poller in pollers {
            poller.stop().await?;
        }
    }

    Ok(())
}

async fn create_orchestrator(token: &str) -> Result<Arc<Orchestrator>> {
    let github_app = Arc::new(GitHubApp::new_from_token(token.to_string())?);
    let github_ops = Arc::new(GitHubOperations::new(Arc::clone(&github_app)));

    let session_id = uuid::Uuid::new_v4().to_string();
    let event_writer = Arc::new(EventWriter::new(EventWriterConfig::default()));
    let budget_tracker = Arc::new(BudgetTracker::new(BudgetConfig::default()).await?);
    let agent_registry = Arc::new(AgentRegistry::new());

    let task_store_path = PathBuf::from(".arkavo/orchestrator-tasks.db");
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
