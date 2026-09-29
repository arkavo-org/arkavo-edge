//! A message answered by an agent that runs no agent loop.
//!
//! The handler executes the task itself, writes the outcome to the task the
//! requester polls, and tells the commander over gossip so it need not poll.

use std::sync::Arc;

use arkavo_protocol::types::{Message, MessagePart, TaskError, TaskStatus};
use arkavo_tasks::task_store::TaskStore;
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::direct::{DirectExecution, Request};
use crate::server::tool_memory::ToolMemory;

pub(super) async fn run(
    direct: DirectExecution,
    task_store: Arc<dyn TaskStore>,
    agent_memory: Arc<RwLock<ToolMemory>>,
    task_id: uuid::Uuid,
    content: String,
    images: Option<Vec<String>>,
) {
    let task_executor = direct.task_executor.clone();
    let specialist_id = direct.agent_metadata.read().await.name.clone();
    let task_start = std::time::Instant::now();

    if let Err(e) = task_executor
        .update_task_status(&task_id, TaskStatus::Working)
        .await
    {
        warn!("Failed to update task {} to Working: {}", task_id, e);
        return;
    }

    info!("Executing task {} via HRM Conductor", task_id);

    let answer = direct
        .answer(Request {
            content,
            images,
            task_id: Some(task_id),
            memory: agent_memory,
            // Specialists may need complexity assessment.
            skip_complexity: false,
        })
        .await;
    let succeeded = answer.is_ok();
    let reported = match answer {
        Ok(result_content) => {
            let result_message = Message {
                parts: vec![MessagePart::Text {
                    content: result_content.clone(),
                }],
                metadata: None,
            };
            let result_value =
                serde_json::to_value(&result_message).unwrap_or(serde_json::Value::Null);

            if let Err(e) = task_executor.complete_task(&task_id, result_value).await {
                warn!("Failed to complete task {}: {}", task_id, e);
                return;
            }
            info!("Task {} completed successfully via HRM", task_id);
            result_content
        }
        Err(error_msg) => {
            let error = TaskError {
                code: "HRM_EXECUTION_ERROR".to_string(),
                message: error_msg.clone(),
                details: None,
            };

            if let Ok(Some(mut task)) = task_store.get_task(&task_id).await {
                task.error = Some(error);
                let _ = task_store.create_task(task).await;
            }

            if let Err(e) = task_executor
                .update_task_status(&task_id, TaskStatus::Failed)
                .await
            {
                warn!("Failed to mark task {} as failed: {}", task_id, e);
                return;
            }
            warn!("Task {} failed: {}", task_id, error_msg);
            error_msg
        }
    };

    // Push the outcome to the commander via gossip
    if let Some(bus) = &direct.learning_bus {
        let snapshot = {
            let budget = direct.compute_budget.read().await;
            serde_json::to_value(budget.snapshot()).ok()
        };
        let notice = arkavo_gossip::TaskCompletionNotice {
            task_id: task_id.to_string(),
            specialist_id,
            succeeded,
            content: reported,
            budget_snapshot: snapshot,
            completion_ms: task_start.elapsed().as_millis() as u64,
        };
        bus.broadcast_to_peers(arkavo_gossip::GossipMessage::TaskCompleted(notice))
            .await;
    }
}
