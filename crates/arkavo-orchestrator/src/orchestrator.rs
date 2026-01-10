use crate::agent_assignment::AgentAssigner;
use crate::cognitive_engine::CognitiveEngine;
use crate::error::{Error, Result};
use crate::issue_router::{ExecutionStrategy, IssueRouter};
use crate::types::IssueEvent;
use arkavo_budget::BudgetTracker;
use arkavo_events::EventWriter;
use arkavo_github::IssueOperations;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_memory::{IssueProcessingStatus, MemoryStorage, OrchestratorStateStore, PlanStateStore};
use arkavo_protocol::{
    agent_registry::AgentRegistry,
    task_executor::TaskExecutor,
    types::{Message, MessagePart, TaskStatus},
};
use arkavo_router::Router;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

const MAX_RETRY_ATTEMPTS: u32 = 3;

pub struct Orchestrator {
    task_executor: Arc<TaskExecutor>,
    agent_assigner: Arc<AgentAssigner>,
    cognitive_engine: Arc<CognitiveEngine>,
    github_ops: Arc<IssueOperations>,
    state_store: Arc<OrchestratorStateStore>,
}

impl Orchestrator {
    pub async fn new(
        task_executor: Arc<TaskExecutor>,
        agent_registry: Arc<AgentRegistry>,
        budget_tracker: Arc<BudgetTracker>,
        event_writer: Arc<EventWriter>,
        github_ops: Arc<IssueOperations>,
        session_id: String,
    ) -> Result<Self> {
        let agent_assigner = Arc::new(AgentAssigner::new(agent_registry));

        let router = Arc::new(
            Router::new()
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!("Failed to initialize router: {e}")))?,
        );

        let storage = Arc::new(
            MemoryStorage::new()
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!("Failed to initialize storage: {e}")))?,
        );

        let tool_registry = Arc::new(ToolRegistry::new(storage));

        // Create plan state store for persistence
        let plan_store = {
            let db_path = dirs::data_local_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("arkavo")
                .join("orchestrator_plans.db");

            match PlanStateStore::new(&db_path).await {
                Ok(store) => {
                    // Mark any "executing" plans as interrupted (orchestrator was restarted)
                    if let Err(e) = store.mark_executing_as_interrupted().await {
                        warn!(error = %e, "Failed to mark executing plans as interrupted");
                    }
                    Some(Arc::new(store))
                }
                Err(e) => {
                    warn!(error = %e, "Failed to create plan state store, proceeding without persistence");
                    None
                }
            }
        };

        let cognitive_engine = Arc::new(CognitiveEngine::new(
            budget_tracker,
            event_writer,
            Arc::clone(&github_ops),
            router,
            tool_registry,
            session_id,
            plan_store,
        ));

        // Create orchestrator state store for persistence
        let state_store =
            {
                let db_path = dirs::data_local_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("arkavo")
                    .join("orchestrator_state.db");

                Arc::new(OrchestratorStateStore::new(&db_path).await.map_err(|e| {
                    Error::Other(anyhow::anyhow!("Failed to create state store: {e}"))
                })?)
            };

        // Resume any processing issues on startup (mark as pending to re-queue)
        if let Ok(processing) = state_store
            .get_issues_by_status(IssueProcessingStatus::Processing)
            .await
        {
            for issue in processing {
                if let Err(e) = state_store
                    .update_issue_status(
                        &issue.org,
                        &issue.repo_name,
                        issue.issue_number,
                        IssueProcessingStatus::Pending,
                        Some("Orchestrator restarted during processing"),
                    )
                    .await
                {
                    warn!(error = %e, "Failed to mark processing issue as pending on startup");
                }
            }
        }

        Ok(Self {
            task_executor,
            agent_assigner,
            cognitive_engine,
            github_ops,
            state_store,
        })
    }

    pub async fn initialize(&self) -> Result<()> {
        info!("Initializing GitHub orchestrator");

        self.task_executor
            .start()
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to start task executor: {e}")))?;

        info!("Task executor started, ready to process GitHub events");
        Ok(())
    }

    pub async fn handle_issue_event(&self, event: IssueEvent) -> Result<()> {
        info!(
            issue = event.issue.number,
            repository = %event.repository.full_name,
            "Processing issue event"
        );

        // Parse org/repo from full_name (e.g., "owner/repo")
        let parts: Vec<&str> = event.repository.full_name.split('/').collect();
        if parts.len() != 2 {
            return Err(Error::Other(anyhow::anyhow!(
                "Invalid repository format: {}",
                event.repository.full_name
            )));
        }
        let (org, repo_name) = (parts[0].to_string(), parts[1].to_string());
        let issue_number = event.issue.number;

        // Check if issue is already being processed
        if let Ok(Some(_task_id)) = self
            .state_store
            .get_issue_task_id(&org, &repo_name, issue_number)
            .await
        {
            // Check if it's still in processing state
            if let Ok(issues) = self
                .state_store
                .get_issues_by_status(IssueProcessingStatus::Processing)
                .await
            {
                let is_processing = issues.iter().any(|i| {
                    i.org == org && i.repo_name == repo_name && i.issue_number == issue_number
                });
                if is_processing {
                    info!(
                        issue = issue_number,
                        repository = %event.repository.full_name,
                        "Issue already has active task, skipping"
                    );
                    return Ok(());
                }
            }
        }

        let routing_decision = IssueRouter::route(&event);

        info!(
            strategy = ?routing_decision.strategy,
            priority = ?routing_decision.priority,
            "Issue routed"
        );

        let assignment = self
            .agent_assigner
            .assign(&event, routing_decision.clone())
            .await?;

        self.post_acknowledgment(&event, &routing_decision.strategy)
            .await?;

        let message = Message {
            parts: vec![MessagePart::Text {
                content: format!(
                    "GitHub Issue: {}\n\nRepository: {}\nIssue #{}: {}\n\n{}",
                    event.repository.full_name,
                    event.repository.full_name,
                    event.issue.number,
                    event.issue.title,
                    event.issue.body.as_deref().unwrap_or("")
                ),
            }],
            metadata: Some(serde_json::json!({
                "github_issue": event.issue.number,
                "repository": event.repository.full_name,
                "strategy": format!("{:?}", routing_decision.strategy),
                "complexity": format!("{:?}", routing_decision.analysis.complexity),
            })),
        };

        let task_id = self
            .task_executor
            .submit_task(message)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to submit task: {e}")))?;

        // Mark issue as processing in state store
        self.state_store
            .mark_issue_processed(
                &org,
                &repo_name,
                issue_number,
                task_id,
                IssueProcessingStatus::Processing,
            )
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to persist issue state: {e}")))?;

        match routing_decision.strategy {
            ExecutionStrategy::AutoExecute | ExecutionStrategy::PlanFirst => {
                self.task_executor
                    .update_task_status(&task_id, TaskStatus::Working)
                    .await
                    .map_err(|e| Error::Other(anyhow::anyhow!("Failed to start task: {e}")))?;

                let cognitive_engine = Arc::clone(&self.cognitive_engine);
                let task_executor = Arc::clone(&self.task_executor);
                let state_store = Arc::clone(&self.state_store);
                let org_clone = org.clone();
                let repo_clone = repo_name.clone();

                tokio::spawn(async move {
                    let result = cognitive_engine.execute(assignment).await;

                    match result {
                        Ok(execution_result) => {
                            if execution_result.success {
                                info!(task_id = %task_id, "Task completed successfully");

                                let completion_message = Message {
                                    parts: vec![MessagePart::Text {
                                        content: execution_result.final_comment,
                                    }],
                                    metadata: Some(serde_json::json!({
                                        "steps_completed": execution_result.steps_completed,
                                        "tokens_used": execution_result.total_tokens_used,
                                    })),
                                };

                                if let Err(e) = task_executor
                                    .complete_task(&task_id, completion_message)
                                    .await
                                {
                                    error!(task_id = %task_id, error = %e, "Failed to mark task complete");
                                }

                                // Mark issue as completed in state store
                                if let Err(e) = state_store
                                    .update_issue_status(
                                        &org_clone,
                                        &repo_clone,
                                        issue_number,
                                        IssueProcessingStatus::Completed,
                                        None,
                                    )
                                    .await
                                {
                                    error!(error = %e, "Failed to update issue status to completed");
                                }
                            } else {
                                warn!(task_id = %task_id, "Task execution incomplete");

                                let retry_count = state_store
                                    .increment_retry_count(&org_clone, &repo_clone, issue_number)
                                    .await
                                    .unwrap_or(MAX_RETRY_ATTEMPTS);

                                if retry_count < MAX_RETRY_ATTEMPTS {
                                    info!(task_id = %task_id, retry_count, "Retrying task");
                                    if let Err(e) = task_executor
                                        .update_task_status(&task_id, TaskStatus::Submitted)
                                        .await
                                    {
                                        error!(task_id = %task_id, error = %e, "Failed to retry task");
                                    }
                                } else {
                                    error!(task_id = %task_id, "Task exceeded max retries");
                                    let error = arkavo_protocol::types::TaskError {
                                        code: "MAX_RETRIES_EXCEEDED".to_string(),
                                        message: "Task failed after maximum retry attempts"
                                            .to_string(),
                                        details: Some(serde_json::json!({
                                            "retry_count": retry_count,
                                            "steps_completed": execution_result.steps_completed,
                                        })),
                                    };
                                    if let Err(e) = task_executor.fail_task(&task_id, error).await {
                                        error!(task_id = %task_id, error = %e, "Failed to mark task failed");
                                    }

                                    // Mark issue as failed in state store
                                    if let Err(e) = state_store
                                        .update_issue_status(
                                            &org_clone,
                                            &repo_clone,
                                            issue_number,
                                            IssueProcessingStatus::Failed,
                                            Some("Task exceeded max retries"),
                                        )
                                        .await
                                    {
                                        error!(error = %e, "Failed to update issue status to failed");
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            error!(task_id = %task_id, error = %e, "Task execution failed");

                            let retry_count = state_store
                                .increment_retry_count(&org_clone, &repo_clone, issue_number)
                                .await
                                .unwrap_or(MAX_RETRY_ATTEMPTS);

                            if retry_count < MAX_RETRY_ATTEMPTS {
                                info!(task_id = %task_id, retry_count, "Retrying task after error");
                                if let Err(e) = task_executor
                                    .update_task_status(&task_id, TaskStatus::Submitted)
                                    .await
                                {
                                    error!(task_id = %task_id, error = %e, "Failed to retry task");
                                }
                            } else {
                                let err_msg = format!("Task failed: {e}");
                                let error = arkavo_protocol::types::TaskError {
                                    code: "EXECUTION_ERROR".to_string(),
                                    message: err_msg.clone(),
                                    details: Some(serde_json::json!({
                                        "retry_count": retry_count,
                                    })),
                                };
                                if let Err(e) = task_executor.fail_task(&task_id, error).await {
                                    error!(task_id = %task_id, error = %e, "Failed to mark task failed");
                                }

                                // Mark issue as failed in state store
                                if let Err(e) = state_store
                                    .update_issue_status(
                                        &org_clone,
                                        &repo_clone,
                                        issue_number,
                                        IssueProcessingStatus::Failed,
                                        Some(&err_msg),
                                    )
                                    .await
                                {
                                    error!(error = %e, "Failed to update issue status to failed");
                                }
                            }
                        }
                    }
                });
            }
            ExecutionStrategy::OrchestratorConsultation => {
                info!(task_id = %task_id, "Task requires orchestrator consultation");
                self.task_executor
                    .update_task_status(&task_id, TaskStatus::InputRequired)
                    .await
                    .map_err(|e| Error::Other(anyhow::anyhow!("Failed to pause task: {e}")))?;
            }
            ExecutionStrategy::HumanApprovalRequired => {
                info!(task_id = %task_id, "Task requires human approval");
                self.task_executor
                    .update_task_status(&task_id, TaskStatus::AuthRequired)
                    .await
                    .map_err(|e| Error::Other(anyhow::anyhow!("Failed to require auth: {e}")))?;
            }
        }

        Ok(())
    }

    async fn post_acknowledgment(
        &self,
        event: &IssueEvent,
        strategy: &ExecutionStrategy,
    ) -> Result<()> {
        let parts: Vec<&str> = event.repository.full_name.split('/').collect();
        if parts.len() != 2 {
            return Err(Error::Other(anyhow::anyhow!("Invalid repository format")));
        }
        let (owner, repo) = (parts[0], parts[1]);

        let message = match strategy {
            ExecutionStrategy::AutoExecute => "🤖 Acknowledged! Auto-executing this task.\n\n\
                **Strategy**: AutoExecute\n\n\
                I'll provide updates as I make progress."
                .to_string(),
            ExecutionStrategy::PlanFirst => {
                "🤖 Acknowledged! Planning implementation approach.\n\n\
                **Strategy**: PlanFirst\n\n\
                I'll create a detailed plan before executing."
                    .to_string()
            }
            ExecutionStrategy::OrchestratorConsultation => {
                "🤖 Acknowledged! This task requires multi-agent coordination.\n\n\
                **Strategy**: OrchestratorConsultation\n\n\
                Coordinating with specialized agents."
                    .to_string()
            }
            ExecutionStrategy::HumanApprovalRequired => {
                "🤖 Acknowledged! This task requires human review.\n\n\
                **Strategy**: HumanApprovalRequired\n\n\
                Please review and approve before I proceed."
                    .to_string()
            }
        };

        self.github_ops
            .post_comment(owner, repo, event.issue.number, &message)
            .await?;
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        info!("Stopping GitHub orchestrator");

        self.task_executor
            .stop()
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to stop task executor: {e}")))?;

        Ok(())
    }

    pub async fn get_task_id_for_issue(&self, repository: &str, issue_number: u64) -> Option<Uuid> {
        let parts: Vec<&str> = repository.split('/').collect();
        if parts.len() != 2 {
            return None;
        }
        let (org, repo_name) = (parts[0], parts[1]);

        self.state_store
            .get_issue_task_id(org, repo_name, issue_number)
            .await
            .ok()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_retry_constant() {
        assert_eq!(MAX_RETRY_ATTEMPTS, 3);
    }
}
