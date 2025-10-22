#![allow(clippy::disallowed_methods)]

use arkavo_protocol::auth::{AuthBackend, JwtAuthBackend, SessionAuth};
use arkavo_protocol::chat_session::ChatSessionManager;
use arkavo_protocol::types::{ClientMetrics, MetricsAck};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
struct TestClaims {
    sub: String,
    exp: i64,
    scopes: Vec<String>,
}

/// Test JWT authentication in chat_open
#[tokio::test]
async fn test_jwt_authentication_success() {
    // Create JWT backend
    let secret = "test_secret_key";
    let auth_backend = JwtAuthBackend::new(secret);

    // Create valid JWT token
    let claims = TestClaims {
        sub: "user123".to_string(),
        exp: chrono::Utc::now().timestamp() + 3600,
        scopes: vec!["chat.read".to_string(), "chat.write".to_string()],
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_ref()),
    )
    .unwrap();

    // Validate token
    let result = auth_backend.validate_token(&token).await;
    assert!(result.is_ok());

    let auth = result.unwrap();
    assert_eq!(auth.sub, "user123");
    assert_eq!(auth.scopes.len(), 2);
    assert!(auth.scopes.contains(&"chat.read".to_string()));
}

/// Test JWT authentication failure with invalid token
#[tokio::test]
async fn test_jwt_authentication_failure() {
    let secret = "test_secret_key";
    let auth_backend = JwtAuthBackend::new(secret);

    // Invalid token
    let invalid_token = "invalid.jwt.token";

    let result = auth_backend.validate_token(invalid_token).await;
    assert!(result.is_err());
}

/// Test expired JWT token
#[tokio::test]
async fn test_jwt_authentication_expired() {
    let secret = "test_secret_key";
    let auth_backend = JwtAuthBackend::new(secret);

    // Create expired JWT token
    let claims = TestClaims {
        sub: "user123".to_string(),
        exp: chrono::Utc::now().timestamp() - 3600, // Expired 1 hour ago
        scopes: vec!["chat.read".to_string()],
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_ref()),
    )
    .unwrap();

    let result = auth_backend.validate_token(&token).await;
    assert!(result.is_err());
}

/// Test chat session creation with authentication
#[tokio::test]
async fn test_authenticated_chat_session() {
    // Create chat session manager without LLM adapter for testing
    let manager = ChatSessionManager::new(None);

    // Create authentication info
    let auth = SessionAuth {
        sub: "user456".to_string(),
        scopes: vec!["chat.full".to_string()],
        exp: Some(chrono::Utc::now().timestamp() + 3600),
        metadata: None,
    };

    // Create session with authentication
    let session = manager.create_session(Some(auth.clone())).await;
    assert!(!session.session_id.is_empty());

    // Verify auth is stored
    let stored_auth = manager.get_session_auth(&session.session_id).await;
    assert!(stored_auth.is_some());

    let stored_auth = stored_auth.unwrap();
    assert_eq!(stored_auth.sub, "user456");
    assert_eq!(stored_auth.scopes, vec!["chat.full"]);
}

/// Test back-pressure management with MetricsAck
#[tokio::test]
async fn test_backpressure_with_metrics_ack() {
    let manager = ChatSessionManager::new(None);

    // Create a session
    let session = manager.create_session(None).await;
    let session_id = session.session_id.clone();

    // Process some deltas (simulating message flow)
    // In real scenario, deltas would be sent through the session

    // Send metrics acknowledgment
    let result = manager.process_metrics_ack(&session_id, 50).await;
    assert!(result.is_ok());

    // Test with non-existent session
    let result = manager.process_metrics_ack("non-existent", 10).await;
    assert!(result.is_err());
}

/// Test tool call delta streaming
#[tokio::test]
async fn test_tool_call_delta_structure() {
    use arkavo_protocol::types::MessageDeltaContent;

    // Create a tool call delta with initial call
    let initial_delta = MessageDeltaContent::ToolCall {
        tool_call_id: "call_123".to_string(),
        name: Some("get_weather".to_string()),
        args_json_fragment: r#"{"location":"#.to_string(),
        done: false,
    };

    // Serialize and verify structure
    let json = serde_json::to_string(&initial_delta).unwrap();
    assert!(json.contains("get_weather"));
    assert!(json.contains("call_123"));
    assert!(json.contains(r#""done":false"#));

    // Create a continuing delta
    let continue_delta = MessageDeltaContent::ToolCall {
        tool_call_id: "call_123".to_string(),
        name: None, // Not sent on continuation
        args_json_fragment: r#"New York"}"#.to_string(),
        done: true,
    };

    let json = serde_json::to_string(&continue_delta).unwrap();
    assert!(!json.contains(r#""name""#)); // name should be omitted when None
    assert!(json.contains(r#""done":true"#));
}

/// Test session persistence
#[tokio::test]
async fn test_session_persistence() {
    use arkavo_protocol::session_persistence::{
        PersistedMessage, PersistedSession, SessionPersistence, SqliteSessionPersistence,
    };
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    let persistence = SqliteSessionPersistence::new(&db_path).await.unwrap();

    // Create and save a session
    let session = PersistedSession {
        session_id: "test-session-123".to_string(),
        auth: Some(SessionAuth {
            sub: "user789".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            exp: None,
            metadata: None,
        }),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        conversation_context: vec![
            "User: Hello".to_string(),
            "Assistant: Hi there!".to_string(),
        ],
    };

    persistence.save_session(&session).await.unwrap();

    // Load the session back
    let loaded = persistence.load_session("test-session-123").await.unwrap();
    assert!(loaded.is_some());

    let loaded = loaded.unwrap();
    assert_eq!(loaded.session_id, "test-session-123");
    assert!(loaded.auth.is_some());
    assert_eq!(loaded.auth.unwrap().sub, "user789");
    assert_eq!(loaded.conversation_context.len(), 2);

    // Save a message
    let message = PersistedMessage {
        session_id: "test-session-123".to_string(),
        message_id: "msg-001".to_string(),
        content: "What's the weather?".to_string(),
        role: "user".to_string(),
        timestamp: chrono::Utc::now(),
    };

    persistence.save_message(&message).await.unwrap();

    // Load messages
    let messages = persistence.load_messages("test-session-123").await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "What's the weather?");

    // Test cleanup
    let old_time = chrono::Utc::now() + chrono::Duration::hours(25);
    let deleted = persistence.cleanup_old_sessions(old_time).await.unwrap();
    assert_eq!(deleted, 1);

    // Verify session was deleted
    let loaded = persistence.load_session("test-session-123").await.unwrap();
    assert!(loaded.is_none());
}

/// Integration test for full chat flow with auth and back-pressure
#[tokio::test]
async fn test_full_chat_flow_integration() {
    // This test simulates a full chat flow:
    // 1. Client authenticates with JWT
    // 2. Opens a chat session
    // 3. Sends messages
    // 4. Receives tool call deltas
    // 5. Sends metrics acknowledgments for back-pressure

    let secret = "integration_test_secret";
    let auth_backend = Arc::new(JwtAuthBackend::new(secret));

    // Create JWT token
    let claims = TestClaims {
        sub: "integration_user".to_string(),
        exp: chrono::Utc::now().timestamp() + 3600,
        scopes: vec!["chat.full".to_string()],
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_ref()),
    )
    .unwrap();

    // Validate and get auth
    let auth = auth_backend.validate_token(&token).await.unwrap();

    // Create chat session manager
    let manager = ChatSessionManager::new(None);

    // Create authenticated session
    let session = manager.create_session(Some(auth)).await;
    let session_id = session.session_id.clone();

    // Verify session exists and is active
    assert!(manager.session_exists(&session_id).await);

    // Simulate sending metrics acknowledgment
    let ack = MetricsAck {
        session_id: session_id.clone(),
        last_seq: 10,
        client_metrics: Some(ClientMetrics {
            buffer_size: 5,
            processing_latency_ms: Some(50),
            ready_for_more: true,
        }),
    };

    // Process the acknowledgment
    let result = manager
        .process_metrics_ack(&ack.session_id, ack.last_seq)
        .await;
    assert!(result.is_ok());

    // Close the session
    let result = manager.close_session(&session_id).await;
    assert!(result.is_ok());

    // Verify session is no longer active
    assert!(!manager.session_exists(&session_id).await);
}

/// Test concurrent session management
#[tokio::test]
async fn test_concurrent_sessions() {
    let manager = Arc::new(ChatSessionManager::new(None));

    // Create multiple sessions concurrently
    let mut handles = vec![];

    for i in 0..10 {
        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            let auth = SessionAuth {
                sub: format!("user_{i}"),
                scopes: vec!["chat".to_string()],
                exp: None,
                metadata: None,
            };

            let session = manager_clone.create_session(Some(auth)).await;
            (i, session.session_id)
        });
        handles.push(handle);
    }

    // Wait for all sessions to be created
    let mut session_ids = vec![];
    for handle in handles {
        let (index, session_id) = handle.await.unwrap();
        session_ids.push((index, session_id));
    }

    // Verify all sessions exist
    for (_, session_id) in &session_ids {
        assert!(manager.session_exists(session_id).await);
    }

    // Send metrics acks concurrently
    let mut ack_handles = vec![];
    for (index, session_id) in session_ids {
        let manager_clone = manager.clone();
        let handle = tokio::spawn(async move {
            let result = manager_clone
                .process_metrics_ack(&session_id, index as u64 * 10)
                .await;
            result.is_ok()
        });
        ack_handles.push(handle);
    }

    // Verify all acks were processed successfully
    for handle in ack_handles {
        assert!(handle.await.unwrap());
    }
}
