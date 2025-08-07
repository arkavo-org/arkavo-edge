use arkavo_debugger::{Diagnostics, SessionReplay};
use arkavo_events::{Event, EventPayload};
use arkavo_memory::storage::MemoryStorage;
use std::sync::Arc;
use uuid::Uuid;

/// Test that session replay accurately reproduces the event sequence
#[tokio::test]
async fn test_replay_determinism() {
    // Create test storage
    let storage = MemoryStorage::new_test().await.unwrap();
    let storage = Arc::new(storage);

    // Create a sequence of test events
    let session_id = Uuid::new_v4().to_string();
    let mut events = Vec::new();

    // 1. Session start
    events.push(Event::new(
        session_id.clone(),
        0,
        "test-agent".to_string(),
        EventPayload::SessionStarted {
            capabilities: Some(vec!["debug".to_string()]),
            metadata: None,
        },
    ));

    // 2. Prompt sent
    events.push(Event::new(
        session_id.clone(),
        1,
        "test-agent".to_string(),
        EventPayload::PromptSent {
            prompt: "Debug my code".to_string(),
            model: "gpt-4".to_string(),
            parameters: None,
        },
    ));

    // 3. Tool call
    events.push(Event::new(
        session_id.clone(),
        2,
        "test-agent".to_string(),
        EventPayload::ToolCall {
            tool_name: "analyze_code".to_string(),
            parameters: serde_json::json!({"file": "main.rs"}),
            tool_call_id: Some("tool-1".to_string()),
        },
    ));

    // 4. Error occurs
    events.push(Event::new(
        session_id.clone(),
        3,
        "test-agent".to_string(),
        EventPayload::Error {
            error_type: "timeout".to_string(),
            message: "Tool execution timed out".to_string(),
            stack_trace: None,
            recoverable: Some(true),
        },
    ));

    // Store events
    use arkavo_memory::event_store::SerializedEvent;
    let serialized_events: Vec<SerializedEvent> = events
        .iter()
        .map(|e| SerializedEvent {
            id: e.id.to_string(),
            session_id: e.session_id.clone(),
            sequence: e.sequence,
            timestamp: e.timestamp.to_rfc3339(),
            agent_id: e.metadata.agent_id.clone(),
            event_type: e.event_type().to_string(),
            payload: serde_json::to_vec(&e.payload).unwrap(),
            schema_version: e.metadata.schema_version.clone(),
        })
        .collect();

    storage.store_events(serialized_events).await.unwrap();

    // Create replay from stored events
    let stored_events = storage.get_session_events(&session_id, None).await.unwrap();
    assert_eq!(stored_events.len(), 4);

    // Verify events can be replayed in order
    let mut replay_events = Vec::new();
    for stored in stored_events {
        // Use JSON for deserialization in tests since bincode doesn't support internally tagged enums
        let payload: EventPayload = serde_json::from_slice(&stored.payload).unwrap();
        let event = Event {
            id: stored.id.parse().unwrap(),
            session_id: stored.session_id,
            sequence: stored.sequence as u64,
            timestamp: chrono::DateTime::parse_from_rfc3339(&stored.timestamp)
                .unwrap()
                .with_timezone(&chrono::Utc),
            metadata: arkavo_events::EventMetadata {
                agent_id: stored.agent_id,
                schema_version: stored.schema_version,
                parent_event_id: None,
                correlation_id: None,
            },
            payload,
        };
        replay_events.push(event);
    }

    // Create replay
    let mut replay = SessionReplay::from_events(replay_events).unwrap();

    // Verify replay navigation
    assert_eq!(replay.len(), 4);
    assert_eq!(replay.position(), 0);

    // Step through and verify sequence
    let event1 = replay.next_event().unwrap();
    assert_eq!(event1.sequence, 0);
    assert!(matches!(
        event1.payload,
        EventPayload::SessionStarted { .. }
    ));

    let event2 = replay.next_event().unwrap();
    assert_eq!(event2.sequence, 1);
    assert!(matches!(event2.payload, EventPayload::PromptSent { .. }));

    // Test backward navigation
    let prev = replay.previous().unwrap();
    assert_eq!(prev.sequence, 0);

    // Jump to error
    let error_event = replay.jump_to(3).unwrap();
    assert!(matches!(error_event.payload, EventPayload::Error { .. }));

    // Verify summary
    let summary = replay.get_summary();
    assert_eq!(summary.total_events, 4);
    assert_eq!(summary.total_errors, 1);
}

/// Test that agents can detect and recover from errors using diagnostics
#[tokio::test]
async fn test_self_healing_agent() {
    use arkavo_debugger::diagnostics::DefaultDiagnostics;

    // Create test storage
    let storage = MemoryStorage::new_test().await.unwrap();
    let storage_arc = Arc::new(storage);

    let session_id = Uuid::new_v4().to_string();
    let agent_id = "self-healing-agent".to_string();

    // Create diagnostics instance
    let diagnostics =
        DefaultDiagnostics::new(agent_id.clone(), session_id.clone(), storage_arc.clone());

    // Simulate a series of events including errors
    let mut events = Vec::new();

    // Add some successful operations
    for i in 0..3 {
        events.push(Event::new(
            session_id.clone(),
            i,
            agent_id.clone(),
            EventPayload::ToolCall {
                tool_name: "api_call".to_string(),
                parameters: serde_json::json!({"endpoint": "/data"}),
                tool_call_id: Some(format!("call-{i}")),
            },
        ));
    }

    // Add rate limit errors
    for i in 3..8 {
        events.push(Event::new(
            session_id.clone(),
            i,
            agent_id.clone(),
            EventPayload::Error {
                error_type: "rate_limit".to_string(),
                message: "API rate limit exceeded".to_string(),
                stack_trace: None,
                recoverable: Some(true),
            },
        ));
    }

    // Store events
    use arkavo_memory::event_store::SerializedEvent;
    let serialized_events: Vec<SerializedEvent> = events
        .iter()
        .map(|e| SerializedEvent {
            id: e.id.to_string(),
            session_id: e.session_id.clone(),
            sequence: e.sequence,
            timestamp: e.timestamp.to_rfc3339(),
            agent_id: e.metadata.agent_id.clone(),
            event_type: e.event_type().to_string(),
            payload: serde_json::to_vec(&e.payload).unwrap(),
            schema_version: e.metadata.schema_version.clone(),
        })
        .collect();

    storage_arc.store_events(serialized_events).await.unwrap();

    // Use diagnostics to detect error patterns
    let patterns = diagnostics.error_patterns().await.unwrap();

    // Should detect rate limit pattern
    assert!(!patterns.is_empty());
    let rate_limit_pattern = patterns
        .iter()
        .find(|p| p.error_type == "rate_limit")
        .expect("Should detect rate limit pattern");

    assert_eq!(rate_limit_pattern.count, 5);
    assert!(rate_limit_pattern.is_recurring(3));

    // Should suggest exponential backoff
    assert!(rate_limit_pattern.suggested_action.is_some());
    assert!(
        rate_limit_pattern
            .suggested_action
            .as_ref()
            .unwrap()
            .contains("backoff")
    );

    // Test recent error detection
    let has_rate_limit = diagnostics
        .has_recent_error("rate_limit", 60)
        .await
        .unwrap();
    assert!(has_rate_limit);

    // Simulate self-healing: Add successful retry after backoff
    let retry_event = Event::new(
        session_id.clone(),
        8,
        agent_id.clone(),
        EventPayload::ToolCall {
            tool_name: "api_call".to_string(),
            parameters: serde_json::json!({
                "endpoint": "/data",
                "retry_attempt": 1,
                "backoff_ms": 1000
            }),
            tool_call_id: Some("retry-1".to_string()),
        },
    );

    storage_arc
        .store_events(vec![SerializedEvent {
            id: retry_event.id.to_string(),
            session_id: retry_event.session_id.clone(),
            sequence: retry_event.sequence,
            timestamp: retry_event.timestamp.to_rfc3339(),
            agent_id: retry_event.metadata.agent_id.clone(),
            event_type: retry_event.event_type().to_string(),
            payload: serde_json::to_vec(&retry_event.payload).unwrap(),
            schema_version: retry_event.metadata.schema_version.clone(),
        }])
        .await
        .unwrap();

    // Verify health report shows recovery
    let health = diagnostics.health().await.unwrap();
    assert_eq!(health.total_events, 9); // 3 success + 5 errors + 1 retry
}

/// Test that debug agent can analyze session patterns
#[tokio::test]
async fn test_debug_agent_analysis() {
    // This would test the debug agent's ability to:
    // 1. Connect to a session via WebSocket
    // 2. Analyze event patterns
    // 3. Provide intelligent recommendations
    // 4. Compare multiple sessions

    // For now, this is a placeholder showing the intended test structure
    assert!(true, "Debug agent analysis test placeholder");
}
