//! Following a task a mesh agent runs, from submission to its answer.
//!
//! `arkavo task` hands work to a mesh agent and is used to drive automation,
//! so what the agent answered, and the assets it delivered, are the result:
//! a bare "completed" left the caller nothing to act on (CLI-018). The task
//! is followed for as long as it reports change (CLI-019), as `arkavo chat`
//! and the `arkavo ui` gateway follow theirs.

use std::time::Duration;

use arkavo_protocol::task_watch::Watch;
use arkavo_protocol::transport::{A2aRequest, A2aResponse, A2aTransport};
use arkavo_protocol::types::{MessagePart, TaskGetRequest, TaskGetResponse, TaskStatus};

use super::config::{MeshAnswer, TaskResult};
use super::ui::TaskUI;
use crate::error::{Error, Result};

/// The wait between two looks at the task.
const POLL: Duration = Duration::from_secs(2);

/// Poll `task_id` on the agent behind `transport` until it ends. Each new
/// progress message is shown once; a poll the agent's rate limiter turns
/// away is no change, and `watch` ends the wait once the task has reported
/// nothing new for its limit.
pub(super) async fn follow<T: A2aTransport + ?Sized>(
    transport: &T,
    task_id: &str,
    ui: &dyn TaskUI,
    mut watch: Watch,
) -> Result<TaskResult> {
    loop {
        let request = A2aRequest::new(
            "tasks/get",
            serde_json::json!([TaskGetRequest {
                task_id: task_id.to_string(),
            }]),
        );
        let response = transport
            .send_request(request)
            .await
            .map_err(|e| failed("poll task status", e))?;
        match response {
            A2aResponse::Success { result, .. } => {
                let task: TaskGetResponse = serde_json::from_value(result)
                    .map_err(|e| failed("parse task status response", e))?;
                match task.status {
                    TaskStatus::Completed => {
                        ui.success("Task completed successfully!");
                        return Ok(answered(task_id, task));
                    }
                    TaskStatus::Failed => {
                        ui.error("Task failed");
                        let why = task.error.map_or_else(
                            || "Task failed on mesh agent".to_string(),
                            |e| format!("Task failed on mesh agent: {}: {}", e.code, e.message),
                        );
                        return Ok(TaskResult::failure(why));
                    }
                    TaskStatus::Canceled => return Ok(TaskResult::failure("Task was cancelled")),
                    _ => match watch.observe(&task) {
                        Ok(Some(message)) => {
                            ui.progress(
                                &message,
                                task.progress.as_ref().and_then(|p| p.percentage),
                            );
                        }
                        Ok(None) => {}
                        Err(quiet) => return Err(failed("follow task", quiet)),
                    },
                }
            }
            A2aResponse::Error { error, .. } => watch
                .refused(task_id, error.code, &error.message)
                .map_err(|why| failed("poll task status", why))?,
        }
        tokio::time::sleep(POLL).await;
    }
}

/// A completed task's result: the text of the agent's answer, with the
/// whole answer kept for the assets it references.
fn answered(task_id: &str, task: TaskGetResponse) -> TaskResult {
    let Some(message) = task.result else {
        return TaskResult::success("Completed via mesh agent");
    };
    let text = message
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Text { content } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let summary = if text.trim().is_empty() {
        "Completed via mesh agent".to_string()
    } else {
        text
    };
    TaskResult::success(summary).with_answer(MeshAnswer {
        task_id: task_id.to_string(),
        message,
    })
}

fn failed(operation: &str, details: impl std::fmt::Display) -> Error {
    Error::TaskExecution {
        operation: operation.to_string(),
        details: details.to_string(),
    }
}

#[cfg(test)]
mod tests;
