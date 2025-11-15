use crate::agent_assignment::AgentAssignment;
use crate::cognitive_engine_core::ExecutionPlan;
use crate::error::{Error, Result};
use arkavo_events::{Event, EventPayload, EventWriter};
use arkavo_mcp_tools::ToolRegistry;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info};

pub struct PrCreator {
    tool_registry: Arc<ToolRegistry>,
    event_writer: Arc<EventWriter>,
    session_id: String,
    sequence: std::sync::atomic::AtomicU64,
}

impl PrCreator {
    pub fn new(
        tool_registry: Arc<ToolRegistry>,
        event_writer: Arc<EventWriter>,
        session_id: String,
    ) -> Self {
        Self {
            tool_registry,
            event_writer,
            session_id,
            sequence: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub async fn create_pull_request(
        &self,
        assignment: &AgentAssignment,
        plan: &ExecutionPlan,
        steps_completed: usize,
        tokens_used: u32,
    ) -> Result<String> {
        let start_time = Instant::now();
        info!(
            issue = assignment.issue_number,
            repository = %assignment.repository,
            "Creating pull request via MCP tool"
        );

        let issue_number = assignment.issue_number;
        let pr_title = format!("Fix issue #{}: {}", issue_number, assignment.issue_title);

        let pr_body = format!(
            "## Summary\n\nAutonomously resolved issue #{}\n\n\
            ## What Changed\n\n{}\n\n\
            ## Execution Details\n\n\
            - Steps completed: {}/{}\n\
            - Tokens used: {}\n\
            - All verifications passed ✓\n\n\
            ## Plan Executed\n\n{}\n\n\
            Resolves #{}\n",
            issue_number,
            plan.steps
                .iter()
                .enumerate()
                .map(|(i, step)| format!("{}. {}", i + 1, step.description))
                .collect::<Vec<_>>()
                .join("\n"),
            steps_completed,
            plan.steps.len(),
            tokens_used,
            plan.steps
                .iter()
                .enumerate()
                .map(|(i, step)| format!(
                    "### Step {}: {}\n\nCommands:\n{}\n",
                    i + 1,
                    step.description,
                    step.commands
                        .iter()
                        .map(|c| format!("- `{c}`"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
                .collect::<Vec<_>>()
                .join("\n"),
            issue_number
        );

        let params = json!({
            "title": pr_title,
            "body": pr_body,
            "base": "main",
        });

        self.log_event(
            "pr_creation_started",
            &json!({
                "issue_number": issue_number,
                "repository": assignment.repository,
                "title": pr_title,
            }),
        )
        .await;

        let tool = self
            .tool_registry
            .get("github_pr_create")
            .ok_or_else(|| Error::Other(anyhow::anyhow!("github_pr_create tool not found")))?;

        let result = tool
            .execute(params)
            .await
            .map_err(|e| Error::Other(anyhow::anyhow!("MCP tool execution failed: {e}")));

        let duration_ms = start_time.elapsed().as_millis() as u64;

        match result {
            Ok(response) => {
                let pr_url = response["pr_url"]
                    .as_str()
                    .ok_or_else(|| Error::Other(anyhow::anyhow!("PR URL not found in response")))?;

                info!(pr_url, duration_ms, "Pull request created successfully");

                self.log_event(
                    "pr_creation_success",
                    &json!({
                        "issue_number": issue_number,
                        "repository": assignment.repository,
                        "pr_url": pr_url,
                        "duration_ms": duration_ms,
                    }),
                )
                .await;

                Ok(pr_url.to_string())
            }
            Err(e) => {
                error!(
                    error = %e,
                    issue_number,
                    duration_ms,
                    "Pull request creation failed"
                );

                self.log_event(
                    "pr_creation_failed",
                    &json!({
                        "issue_number": issue_number,
                        "repository": assignment.repository,
                        "error": e.to_string(),
                        "duration_ms": duration_ms,
                        "recoverable": true,
                    }),
                )
                .await;

                Err(Error::Other(anyhow::anyhow!(
                    "MCP tool execution failed: {e}"
                )))
            }
        }
    }

    async fn log_event<T: serde::Serialize>(&self, event_type: &str, data: &T) {
        let seq = self
            .sequence
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let payload = EventPayload::ReasoningStep {
            step_type: event_type.to_string(),
            description: serde_json::to_string(data).unwrap_or_default(),
            metadata: Some(serde_json::to_value(data).unwrap_or(serde_json::Value::Null)),
        };

        let event = Event::new(
            self.session_id.clone(),
            seq,
            "github-orchestrator-pr".to_string(),
            payload,
        );

        if let Err(e) = self.event_writer.write(event).await {
            error!(error = %e, "Failed to log PR creation event");
        }
    }
}
