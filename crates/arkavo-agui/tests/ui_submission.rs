use arkavo_agui::AgUiEvent;
use serde_json::json;

#[test]
fn legacy_task_submission_remains_compatible() {
    let event: AgUiEvent = serde_json::from_value(json!({
        "type": "submitTask", "description": "Inspect the build"
    }))
    .unwrap();
    assert!(matches!(
        event,
        AgUiEvent::SubmitTask {
            request_id: None,
            ..
        }
    ));
}

#[test]
fn request_correlation_round_trips_without_changing_task_identity() {
    let request: AgUiEvent = serde_json::from_value(json!({
        "type": "submitTask", "description": "Inspect the build", "requestId": "browser-123"
    }))
    .unwrap();
    let AgUiEvent::SubmitTask { request_id, .. } = request else {
        panic!("expected submission");
    };
    let response = AgUiEvent::TaskSubmitted {
        request_id,
        task_id: "server-task-id".into(),
        description: Some("Inspect the build".into()),
        summary: None,
        status: "submitted".into(),
        source: Some("ui".into()),
        timestamp: "2026-10-17T12:00:00Z".into(),
    };
    let json = serde_json::to_value(response).unwrap();
    assert_eq!(json["requestId"], "browser-123");
    assert_eq!(json["taskId"], "server-task-id");
    assert_eq!(json["type"], "taskSubmitted");
}
