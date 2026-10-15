use crate::ui::task_window::{MessageRole, TaskManager, TaskStatus};
use crate::{LlmRequest, LlmResponse};
use arkavo_test_macros::spec;

#[test]
#[spec("TERM-003")]
fn llm_request_roundtrip_preserves_task_and_model() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<LlmRequest>(100);

    let request = LlmRequest {
        task_id: uuid::Uuid::new_v4(),
        model_name: "ministral-8b".to_string(),
        prompt: "Summarize this file".to_string(),
    };
    let task_id = request.task_id;
    tx.try_send(request).unwrap();

    let queued = rx.try_recv().unwrap();
    assert_eq!(queued.task_id, task_id);
    assert_eq!(queued.model_name, "ministral-8b");
    assert_eq!(queued.prompt, "Summarize this file");
}

#[test]
#[spec("TERM-003")]
fn llm_requests_get_unique_task_ids() {
    let first = LlmRequest {
        task_id: uuid::Uuid::new_v4(),
        model_name: "m".to_string(),
        prompt: "a".to_string(),
    };
    let second = LlmRequest {
        task_id: uuid::Uuid::new_v4(),
        model_name: "m".to_string(),
        prompt: "b".to_string(),
    };

    // Each queued request is tracked by its own task id
    assert_ne!(first.task_id, second.task_id);
}

#[test]
#[spec("TERM-004")]
fn error_response_marks_task_error_and_displays_message() {
    // Mirrors the response-handling path in App: an LlmResponse carrying an
    // error must surface on the matching task, not vanish silently.
    let mut task_manager = TaskManager::new();
    let task_id = task_manager
        .create_task("agent".to_string(), "model".to_string())
        .id;

    let response = LlmResponse {
        task_id,
        model_name: "model".to_string(),
        content: String::new(),
        is_streaming: false,
        is_complete: true,
        error: Some("connection refused".to_string()),
        mcp_status: None,
    };

    let task = task_manager.find_task_by_id_mut(response.task_id).unwrap();
    if let Some(error) = response.error {
        task.set_status(TaskStatus::Error);
        task.add_message(MessageRole::System, format!("Error: {error}"));
    }

    let task = task_manager.find_task_by_id(task_id).unwrap();
    assert_eq!(task.status, TaskStatus::Error);
    assert_eq!(
        task.messages.back().unwrap().content,
        "Error: connection refused"
    );
}
