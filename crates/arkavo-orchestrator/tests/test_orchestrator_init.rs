#![allow(clippy::disallowed_methods)]

use arkavo_budget::{BudgetConfig, BudgetTracker};
use arkavo_events::{EventWriter, EventWriterConfig};
use arkavo_github::{GitHubApp, IssueOperations};
use arkavo_orchestrator::Orchestrator;
use arkavo_protocol::{
    SqliteTaskStore, TaskExecutor, TaskExecutorConfig, TaskStore, agent_registry::AgentRegistry,
};
use arkavo_test_macros::spec;
use std::sync::Arc;

/// ORCH-001: Initialize orchestrator
///
/// Verifies that `Orchestrator::new` builds the orchestrator and that
/// `Orchestrator::initialize` successfully starts the task executor.
#[spec("ORCH-001")]
#[tokio::test]
async fn test_orchestrator_initialization() {
    let store: Arc<dyn TaskStore> = Arc::new(
        SqliteTaskStore::new_in_memory()
            .await
            .expect("in-memory task store"),
    );
    let task_executor = Arc::new(TaskExecutor::new(store, TaskExecutorConfig::default()));
    let agent_registry = Arc::new(AgentRegistry::new());
    let budget_tracker = Arc::new(
        BudgetTracker::new(BudgetConfig::default())
            .await
            .expect("budget tracker"),
    );
    let event_writer = Arc::new(EventWriter::new(EventWriterConfig::default()));
    let github_app = Arc::new(
        GitHubApp::new_from_token("test-token".to_string()).expect("GitHub app from token"),
    );
    let github_ops = Arc::new(IssueOperations::new(github_app));

    let orchestrator = Orchestrator::new(
        task_executor,
        agent_registry,
        budget_tracker,
        event_writer,
        github_ops,
        "test-session".to_string(),
    )
    .await
    .expect("orchestrator should construct successfully");

    orchestrator
        .initialize()
        .await
        .expect("orchestrator initialize should succeed");

    orchestrator
        .stop()
        .await
        .expect("orchestrator stop should succeed");
}
