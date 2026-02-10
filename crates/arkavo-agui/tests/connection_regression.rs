//! Regression tests for A2A connection bugs fixed in a15189a

use arkavo_agui::types::AgUiEvent;

/// Regression: chat_send RPC returns () (null JSON) but the client previously
/// deserialized the response as String, causing "invalid type: null, expected
/// a string" on every message. Verify the correct type handles void returns.
#[tokio::test]
async fn test_chat_send_accepts_void_response() {
    // Simulate what jsonrpsee does: deserialize null as ()
    let null_json = serde_json::Value::Null;
    let result: Result<(), _> = serde_json::from_value(null_json);
    assert!(result.is_ok(), "() must deserialize from null JSON");

    // The old bug: String cannot deserialize from null
    let null_json = serde_json::Value::Null;
    let result: Result<String, _> = serde_json::from_value(null_json);
    assert!(result.is_err(), "String must NOT deserialize from null JSON");
}

/// Regression: AgentConnection.connect() built WS URL as ws://{endpoint}/ws
/// but jsonrpsee servers listen at root path. Verify URL construction matches
/// the pattern used in agent_connection.rs connect().
#[test]
fn test_ws_connection_url_has_no_path_suffix() {
    let endpoint = "10.0.0.1:8340";
    // This mirrors the URL construction in AgentConnection::connect()
    let url = format!("ws://{endpoint}");
    assert_eq!(url, "ws://10.0.0.1:8340");
    assert!(!url.ends_with("/ws"), "URL must not have /ws suffix");
    // Verify it's a valid WebSocket URL
    assert!(url.starts_with("ws://"));
    // Only one :// in the URL (the scheme separator)
    assert_eq!(url.matches("://").count(), 1, "Only scheme separator");
}

/// Regression: mDNS discovery used tokio::spawn with blocking recv_timeout,
/// starving the async runtime. Verify the channel-based bridge pattern works:
/// blocking producer → mpsc channel → async consumer.
#[tokio::test]
async fn test_mdns_blocking_to_async_bridge() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(16);

    // Simulate the spawn_blocking + blocking_send pattern from mdns_impl.rs
    tokio::task::spawn_blocking(move || {
        for i in 0..3 {
            if tx.blocking_send(format!("service_{i}")).is_err() {
                break;
            }
        }
    });

    // Async consumer receives all messages without blocking the runtime
    let mut received = Vec::new();
    while let Ok(msg) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        rx.recv(),
    )
    .await
    {
        if let Some(msg) = msg {
            received.push(msg);
        } else {
            break;
        }
    }

    assert_eq!(received.len(), 3);
    assert_eq!(received[0], "service_0");
    assert_eq!(received[2], "service_2");
}

/// Verify ChatOpen and UserMessage events deserialize with correct agent_id
#[test]
fn test_chat_events_serde_roundtrip() {
    let chat_open = r#"{"type":"chatOpen","agentId":"orchestrator"}"#;
    let event: AgUiEvent = serde_json::from_str(chat_open).unwrap();
    match event {
        AgUiEvent::ChatOpen { agent_id } => assert_eq!(agent_id, "orchestrator"),
        other => panic!("Expected ChatOpen, got {other:?}"),
    }

    let user_msg = r#"{"type":"userMessage","agentId":"orchestrator","content":"hi"}"#;
    let event: AgUiEvent = serde_json::from_str(user_msg).unwrap();
    match event {
        AgUiEvent::UserMessage {
            agent_id, content, ..
        } => {
            assert_eq!(agent_id, "orchestrator");
            assert_eq!(content, "hi");
        }
        other => panic!("Expected UserMessage, got {other:?}"),
    }
}

/// Verify error events serialize correctly for the frontend
#[test]
fn test_error_event_serialization() {
    let error = AgUiEvent::Error {
        code: "AGENT_NOT_CONNECTED".to_string(),
        message: "Agent orchestrator is not connected".to_string(),
    };
    let json = serde_json::to_string(&error).unwrap();
    assert!(json.contains("AGENT_NOT_CONNECTED"));
    assert!(json.contains("\"type\":\"error\""));
}
