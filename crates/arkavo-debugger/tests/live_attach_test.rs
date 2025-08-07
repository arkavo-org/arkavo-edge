use arkavo_debugger::SessionManager;
use arkavo_events::{Event, EventPayload, EventWriterConfig};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn test_live_attach_session_discovery() {
    // Create a session manager
    let session_manager = Arc::new(SessionManager::new());

    // Register multiple agent sessions
    let session1 = "session-1".to_string();
    let session2 = "session-2".to_string();

    session_manager
        .register_session(
            session1.clone(),
            "agent-1".to_string(),
            "TestAgent1".to_string(),
            "localhost:8080".to_string(),
            vec!["capability1".to_string(), "capability2".to_string()],
            None,
        )
        .await
        .unwrap();

    session_manager
        .register_session(
            session2.clone(),
            "agent-2".to_string(),
            "TestAgent2".to_string(),
            "localhost:8081".to_string(),
            vec!["capability3".to_string()],
            None,
        )
        .await
        .unwrap();

    // Get active sessions
    let active_sessions = session_manager.get_active_sessions().await;
    assert_eq!(active_sessions.len(), 2);

    // Verify session details
    let session1_info = active_sessions
        .iter()
        .find(|s| s.session_id == session1)
        .expect("Session 1 not found");

    assert_eq!(session1_info.agent_name, "TestAgent1");
    assert_eq!(session1_info.endpoint, "localhost:8080");
    assert_eq!(session1_info.capabilities.len(), 2);

    // Test session cleanup
    session_manager
        .cleanup_inactive_sessions(chrono::Duration::zero())
        .await;
    let remaining_sessions = session_manager.get_active_sessions().await;
    assert_eq!(remaining_sessions.len(), 0);
}

#[tokio::test]
async fn test_live_attach_event_capture() {
    use arkavo_events::writer::EventWriterBuilder;

    // Create an event writer with test handler
    let events_captured = Arc::new(tokio::sync::RwLock::new(Vec::new()));
    let events_clone = events_captured.clone();

    let config = EventWriterConfig {
        buffer_size: 100,
        flush_interval: Duration::from_millis(10),
        batch_size: 5,
    };

    let event_writer = Arc::new(
        EventWriterBuilder::new()
            .with_config(config)
            .add_handler(move |events| {
                let events_clone = events_clone.clone();
                tokio::spawn(async move {
                    events_clone.write().await.extend(events);
                });
            })
            .build(),
    );

    // Create session manager and register with event writer
    let session_manager = SessionManager::new();
    let session_id = "test-session".to_string();

    session_manager
        .register_session(
            session_id.clone(),
            "agent-test".to_string(),
            "TestAgent".to_string(),
            "localhost:9000".to_string(),
            vec!["debug".to_string()],
            Some(event_writer.clone()),
        )
        .await
        .unwrap();

    // Write some events
    for i in 0..10 {
        let event = Event::new(
            session_id.clone(),
            i,
            "agent-test".to_string(),
            EventPayload::ToolCall {
                tool_name: format!("test_tool_{i}"),
                parameters: serde_json::json!({ "index": i }),
                tool_call_id: Some(format!("call-{i}")),
            },
        );

        session_manager.write_event(event).await.unwrap();
    }

    // Wait for events to be flushed
    sleep(Duration::from_millis(50)).await;

    // Verify events were captured
    let captured = events_captured.read().await;
    assert!(
        captured.len() >= 10,
        "Expected at least 10 events, got {}",
        captured.len()
    );

    // Verify session activity was updated
    let session_info = session_manager.get_session(&session_id).await.unwrap();
    assert_eq!(session_info.event_count, 10);
}

#[tokio::test]
async fn test_live_attach_session_lifecycle() {
    let session_manager = SessionManager::new();

    // Test session not found
    let missing = session_manager.get_session("nonexistent").await;
    assert!(missing.is_none());

    // Register and verify
    let session_id = "lifecycle-test".to_string();
    session_manager
        .register_session(
            session_id.clone(),
            "agent-lifecycle".to_string(),
            "LifecycleAgent".to_string(),
            "localhost:7000".to_string(),
            vec![],
            None,
        )
        .await
        .unwrap();

    let session = session_manager.get_session(&session_id).await;
    assert!(session.is_some());

    // Unregister and verify
    session_manager.unregister_session(&session_id).await;
    let session_after = session_manager.get_session(&session_id).await;
    assert!(session_after.is_none());
}
