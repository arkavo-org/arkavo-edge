//! What a requester polling `tasks/get` sees once its task has finished.
//!
//! Drives the conductor the way `message/send` does when no agent loop is
//! running: execute against the task the requester polls, with the mesh tools
//! a specialist is given, then write the outcome to it.

// The Tokio test entrypoint owns its runtime.
#![allow(clippy::disallowed_methods)]

#[path = "support/model.rs"]
mod model;

use std::sync::Arc;
use std::time::Duration;

use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_mcp_mesh::MeshToolsState;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::types::{Message, MessagePart, TaskStatus};
use arkavo_server::execute_with_conductor_and_learning;
use arkavo_tasks::task_executor::{TaskExecutor, TaskExecutorConfig};
use arkavo_tasks::task_store::{SqliteTaskStore, TaskStore};
use model::{SCRIPTED_MODEL, Script, scripted_router, text};

fn message(content: &str) -> Message {
    Message {
        parts: vec![MessagePart::Text {
            content: content.to_string(),
        }],
        metadata: None,
    }
}

/// Regression: the conductor reported progress from detached tasks, and the
/// last one ("Finalizing") was still in flight when the task was completed.
/// It wrote back the row it had read before the completion, so the task
/// stayed `Working` with no result and the requester polled until its own
/// timeout.
#[tokio::test]
async fn a_completed_task_stays_completed() {
    let question = "Which channel should we cut?";
    let answer = "Organic search drives 62% of signups; paid social converts worst.";
    let script = Script::new(vec![text(answer)]);
    let router = scripted_router(&script).await;
    let store: Arc<dyn TaskStore> = Arc::new(
        SqliteTaskStore::new_in_memory()
            .await
            .expect("in-memory task store"),
    );
    let executor = Arc::new(TaskExecutor::new(
        store.clone(),
        TaskExecutorConfig::default(),
    ));
    let mesh_state = Arc::new(MeshToolsState::new());
    let mut registry = ToolRegistry::empty();
    arkavo_mcp_mesh::register_tools(&mut registry, mesh_state.clone());
    let task_id = executor
        .submit_task(message(question))
        .await
        .expect("task submitted");
    executor
        .update_task_status(&task_id, TaskStatus::Working)
        .await
        .expect("task started");

    let produced = execute_with_conductor_and_learning(
        &Arc::new(Conductor::new(InMemoryTaskStore::new())),
        &router,
        &Arc::new(McpRegistry::new()),
        question.to_string(),
        Some(task_id),
        Some(&executor),
        None,
        None,
        None,
        Some(&mesh_state),
        Some(&SCRIPTED_MODEL),
        None,
        None,
        None,
        true,
        Some(Arc::new(registry)),
        None,
        #[cfg(feature = "iroh")]
        None,
    )
    .await
    .expect("the model answered");
    let result = serde_json::to_value(message(&produced)).expect("result serializes");
    executor
        .complete_task(&task_id, result.clone())
        .await
        .expect("task completed");

    // A requester polls for as long as the task is not terminal, so every
    // read from here on has to show the finished task. The reads go on long
    // enough for any write still in flight to land.
    for poll in 0..50 {
        let task = store
            .get_task(&task_id)
            .await
            .expect("task store readable")
            .expect("task exists");
        assert_eq!(task.status, TaskStatus::Completed, "poll {poll}");
        assert_eq!(task.result.as_ref(), Some(&result), "poll {poll}");
        // The last thing the conductor reported, and not an earlier report
        // that was written late.
        let progress = task.progress.expect("progress recorded");
        assert_eq!(progress.percentage, Some(95), "poll {poll}");
        assert_eq!(
            progress.message.as_deref(),
            Some("Finalizing"),
            "poll {poll}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    assert!(script.dispatches() >= 1, "the model was asked");
    assert!(script.prompt(0).contains(question));
}
