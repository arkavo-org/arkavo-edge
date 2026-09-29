//! Answering one step of someone else's pipeline run.
//!
//! The agent loop keeps one conversation for the life of the agent and folds
//! the messages queued in a tick into one cycle. A step answered there would
//! be answered with earlier runs in front of the model, or together with
//! another run's step. A marked message is answered here instead, from the
//! agent's purpose and the message alone.

use std::time::Duration;

use arkavo_protocol::types::TaskStatus;
use tracing::{info, warn};
use uuid::Uuid;

use crate::server::agent_cycle_reply::{apply_outcome, outcome_for_cycle};
use crate::server::handlers::messaging::direct::{DirectExecution, Request};

/// Answer one step of someone else's run and write the outcome to its task.
///
/// `allowed` is how long the sender said it will wait. Work that outlasts it
/// would be for nobody, so it is stopped there.
pub(in crate::server) async fn answer_step(
    direct: DirectExecution,
    task_id: Uuid,
    content: String,
    allowed: Option<Duration>,
) {
    let executor = direct.task_executor.clone();
    if let Err(e) = executor
        .update_task_status(&task_id, TaskStatus::Working)
        .await
    {
        warn!("Failed to update task {task_id} to Working: {e}");
    }
    info!(task_id = %task_id, "Answering a pipeline step on its own, outside the agent loop");

    let work = direct.answer(Request::isolated(content, Some(task_id)));
    let answer = match allowed {
        Some(allowed) => tokio::time::timeout(allowed, work)
            .await
            .unwrap_or_else(|_| {
                Err(format!(
                    "the pipeline step did not finish within the {}s its sender allowed",
                    allowed.as_secs()
                ))
            }),
        None => work.await,
    };
    apply_outcome(&executor, &task_id, &outcome_for_cycle(&answer)).await;
}
