use crate::agent_assignment::AgentAssigner;
use crate::cognitive_engine::CognitiveEngine;
use crate::error::{Error, Result};
use crate::github_operations::GitHubOperations;
use crate::issue_router::{ExecutionStrategy, IssueRouter};
use crate::types::IssueEvent;
use arkavo_budget::BudgetTracker;
use arkavo_events::EventWriter;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_protocol::{
    agent_registry::AgentRegistry,
    mcp::McpClient,
    task_executor::TaskExecutor,
    types::{Message, MessagePart, TaskStatus},
};
use arkavo_router::Router;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use uuid::Uuid;

const MAX_RETRY_ATTEMPTS: u32 = 3;

pub struct Orchestrator {
    task_executor: Arc<TaskExecutor>,
    agent_assigner: Arc<AgentAssigner>,
    cognitive_engine: Arc<CognitiveEngine>,
    github_ops: Arc<GitHubOperations>,
    issue_to_task: Arc<RwLock<HashMap<String, Uuid>>>,
    task_retry_counts: Arc<RwLock<HashMap<Uuid, u32>>>,
}

impl Orchestrator {
    pub async fn new(
        task_executor: Arc<TaskExecutor>,
        agent_registry: Arc<AgentRegistry>,
        mcp_client: Arc<McpClient>,
        budget_tracker: Arc<BudgetTracker>,
        event_writer: Arc<EventWriter>,
        github_ops: Arc<GitHubOperations>,
        session_id: String,
    ) -> Result<Self> {
        let agent_assigner = Arc::new(AgentAssigner::new(agent_registry));

        let router = Arc::new(
            Router::new()
                .await
                .map_err(|e| Error::Other(anyhow::anyhow!("Failed to initialize router: {e}")))?,
        );

        let tool_registry = Arc::new(ToolRegistry::new());

        let cognitive_engine = Arc::new(CognitiveEngine::new(
            mcp_client,
            budget_tracker,
            event_writer,
            Arc::clone(&github_ops),
            router,
            tool_registry,
            session_id,
        ));

        Ok(Self {
            task_executor,
            agent_assigner,
            cognitive_engine,
            github_ops,
            issue_to_task: Arc::new(RwLock::new(HashMap::new())),
            task_retry_counts: Arc::new(RwLock::new(HashMap::new())),
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

        let issue_key = format!("{}#{}", event.repository.full_name, event.issue.number);

        if self.is_issue_active(&issue_key).await {
            info!(issue_key, "Issue already has active task, skipping");
            return Ok(());
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

        self.issue_to_task
            .write()
            .await
            .insert(issue_key.clone(), task_id);

        match routing_decision.strategy {
            ExecutionStrategy::AutoExecute | ExecutionStrategy::PlanFirst => {
                self.task_executor
                    .update_task_status(&task_id, TaskStatus::Working)
                    .await
                    .map_err(|e| Error::Other(anyhow::anyhow!("Failed to start task: {e}")))?;

                let cognitive_engine = Arc::clone(&self.cognitive_engine);
                let task_executor = Arc::clone(&self.task_executor);
                let task_retry_counts = Arc::clone(&self.task_retry_counts);
                let issue_to_task_map = Arc::clone(&self.issue_to_task);

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
                            } else {
                                warn!(task_id = %task_id, "Task execution incomplete");

                                let retry_count = {
                                    let mut counts = task_retry_counts.write().await;
                                    let count = counts.entry(task_id).or_insert(0);
                                    *count += 1;
                                    *count
                                };

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
                                }
                            }
                        }
                        Err(e) => {
                            error!(task_id = %task_id, error = %e, "Task execution failed");

                            let retry_count = {
                                let mut counts = task_retry_counts.write().await;
                                let count = counts.entry(task_id).or_insert(0);
                                *count += 1;
                                *count
                            };

                            if retry_count < MAX_RETRY_ATTEMPTS {
                                info!(task_id = %task_id, retry_count, "Retrying task after error");
                                if let Err(e) = task_executor
                                    .update_task_status(&task_id, TaskStatus::Submitted)
                                    .await
                                {
                                    error!(task_id = %task_id, error = %e, "Failed to retry task");
                                }
                            } else {
                                let error = arkavo_protocol::types::TaskError {
                                    code: "EXECUTION_ERROR".to_string(),
                                    message: format!("Task failed: {e}"),
                                    details: Some(serde_json::json!({
                                        "retry_count": retry_count,
                                    })),
                                };
                                if let Err(e) = task_executor.fail_task(&task_id, error).await {
                                    error!(task_id = %task_id, error = %e, "Failed to mark task failed");
                                }
                            }
                        }
                    }

                    issue_to_task_map.write().await.remove(&issue_key);
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
            .await
    }

    async fn is_issue_active(&self, issue_key: &str) -> bool {
        self.issue_to_task.read().await.contains_key(issue_key)
    }

    pub async fn stop(&self) -> Result<()> {
        info!("Stopping GitHub orchestrator");

        self.task_executor
            .stop()
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to stop task executor: {e}")))?;

        Ok(())
    }

    pub async fn get_task_id_for_issue(&self, repository: &str, issue_number: u64) -> Option<Uuid> {
        let issue_key = format!("{repository}#{issue_number}");
        self.issue_to_task.read().await.get(&issue_key).copied()
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
