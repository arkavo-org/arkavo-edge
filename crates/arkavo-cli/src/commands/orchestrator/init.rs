use anyhow::Result;
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

pub(super) async fn create_orchestrator(github_app: Arc<GitHubApp>) -> Result<Arc<Orchestrator>> {
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

pub(super) async fn create_polling_orchestrator(token: &str) -> Result<Arc<Orchestrator>> {
    let github_app = Arc::new(GitHubApp::new_from_token(token.to_string())?);
    create_orchestrator(github_app).await
}
