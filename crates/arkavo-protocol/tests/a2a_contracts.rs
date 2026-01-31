//! A2A Protocol Contract Tests
//!
//! These tests verify architectural boundaries:
//! - PROTOCOL-A2A-001: TaskRequest/TaskResponse schema compliance
//! - PROTOCOL-A2A-002: AgentCard well-known format
//! - PROTOCOL-A2A-003: Message serialization round-trip
//! - PROTOCOL-A2A-004: Backward compatibility guarantees

use arkavo_protocol::types::*;
use serde_json::json;
use uuid::Uuid;

/// Contract: TaskRequest must serialize to valid JSON with required fields
#[test]
fn contract_task_request_schema() {
    let request = TaskRequest {
        agent_id: "agent-123".to_string(),
        task_type: "code_review".to_string(),
        payload: Some(json!({"file": "main.rs"})),
    };

    let json_str = serde_json::to_string(&request).expect("Must serialize to JSON");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("Must be valid JSON");

    // Required fields must be present
    assert!(
        parsed.get("agent_id").is_some(),
        "TaskRequest must have agent_id field"
    );
    assert!(
        parsed.get("task_type").is_some(),
        "TaskRequest must have task_type field"
    );

    // Values must match
    assert_eq!(
        parsed["agent_id"], "agent-123",
        "agent_id must match input value"
    );
    assert_eq!(
        parsed["task_type"], "code_review",
        "task_type must match input value"
    );
}

/// Contract: TaskResponse must include task_id and status
#[test]
fn contract_task_response_schema() {
    let task_id = Uuid::new_v4();
    let response = TaskResponse {
        task_id,
        status: TaskStatus::Completed,
        data: Some(json!({"result": "success"})),
    };

    let json_str = serde_json::to_string(&response).expect("Must serialize to JSON");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("Must be valid JSON");

    assert!(
        parsed.get("task_id").is_some(),
        "TaskResponse must have task_id field"
    );
    assert!(
        parsed.get("status").is_some(),
        "TaskResponse must have status field"
    );
    assert_eq!(
        parsed["status"], "completed",
        "Status must serialize to snake_case string"
    );
}

/// Contract: TaskStatus round-trip serialization
#[test]
fn contract_task_status_roundtrip() {
    let statuses = vec![
        TaskStatus::Submitted,
        TaskStatus::Working,
        TaskStatus::InputRequired,
        TaskStatus::Completed,
        TaskStatus::Canceled,
        TaskStatus::Failed,
        TaskStatus::Rejected,
        TaskStatus::AuthRequired,
    ];

    for original in statuses {
        let json_str = serde_json::to_string(&original).expect("Must serialize");
        let restored: TaskStatus = serde_json::from_str(&json_str).expect("Must deserialize");
        assert_eq!(
            original, restored,
            "TaskStatus must round-trip correctly: {:?}",
            original
        );
    }
}

/// Contract: AgentCard must have required fields per A2A spec
#[test]
fn contract_agent_card_required_fields() {
    let card = AgentCard {
        name: "Test Agent".to_string(),
        description: Some("A test agent".to_string()),
        url: "https://example.com/agent".to_string(),
        provider: Some(AgentProvider {
            organization: "Test Org".to_string(),
            url: Some("https://example.com".to_string()),
        }),
        version: "1.0.0".to_string(),
        protocol_versions: vec!["0.3".to_string()],
        default_input_modes: vec!["text".to_string()],
        default_output_modes: vec!["text".to_string()],
        capabilities: AgentCapabilities {
            streaming: true,
            push_notifications: false,
            state_transition_history: true,
        },
        skills: vec![AgentSkill {
            id: "skill-1".to_string(),
            name: "Code Review".to_string(),
            description: Some("Reviews code".to_string()),
            tags: vec!["code".to_string()],
            examples: vec!["Review this Rust code".to_string()],
            input_modes: vec![],
            output_modes: vec![],
        }],
        security_schemes: vec![],
        security: vec![],
        extensions: vec![],
        signature: None,
    };

    let json_str = serde_json::to_string(&card).expect("Must serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("Must be valid JSON");

    // Required fields per A2A spec v0.3
    assert!(
        parsed.get("name").is_some(),
        "AgentCard must have name field"
    );
    assert!(
        parsed.get("url").is_some(),
        "AgentCard must have url field"
    );
    assert!(
        parsed.get("version").is_some(),
        "AgentCard must have version field"
    );
    assert!(
        parsed.get("capabilities").is_some(),
        "AgentCard must have capabilities field"
    );

    // CamelCase field names
    assert!(
        parsed.get("protocolVersions").is_some(),
        "protocolVersions must use camelCase"
    );
    assert!(
        parsed.get("defaultInputModes").is_some(),
        "defaultInputModes must use camelCase"
    );
}

/// Contract: AgentCard round-trip preserves all fields
#[test]
fn contract_agent_card_roundtrip() {
    let original = AgentCard {
        name: "Test Agent".to_string(),
        description: Some("A test agent".to_string()),
        url: "https://example.com/agent".to_string(),
        provider: None,
        version: "1.0.0".to_string(),
        protocol_versions: vec!["0.3".to_string()],
        default_input_modes: vec!["text".to_string()],
        default_output_modes: vec!["text".to_string()],
        capabilities: AgentCapabilities::default(),
        skills: vec![],
        security_schemes: vec![],
        security: vec![],
        extensions: vec![],
        signature: None,
    };

    let json_str = serde_json::to_string(&original).expect("Must serialize");
    let restored: AgentCard = serde_json::from_str(&json_str).expect("Must deserialize");

    assert_eq!(original.name, restored.name);
    assert_eq!(original.url, restored.url);
    assert_eq!(original.version, restored.version);
}

/// Contract: Message parts must serialize with type tag
#[test]
fn contract_message_part_type_tag() {
    let parts = vec![
        MessagePart::Text {
            content: "Hello".to_string(),
        },
        MessagePart::Data {
            schema: "json".to_string(),
            content: json!({"key": "value"}),
        },
    ];

    let message = Message {
        parts,
        metadata: None,
    };

    let json_str = serde_json::to_string(&message).expect("Must serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("Must be valid JSON");

    // Parts must have type field
    let parts = parsed["parts"].as_array().expect("parts must be array");
    assert_eq!(parts.len(), 2);

    for part in parts {
        assert!(
            part.get("type").is_some(),
            "Each MessagePart must have a 'type' field"
        );
    }

    // First part should be "text"
    assert_eq!(parts[0]["type"], "text");
    // Second part should be "data"
    assert_eq!(parts[1]["type"], "data");
}

/// Contract: AgentBroadcast serialization
#[test]
fn contract_agent_broadcast_schema() {
    let broadcast = AgentBroadcast {
        agent_id: "agent-123".to_string(),
        broadcast_type: BroadcastType::Capability,
        capabilities: Some(vec![AgentCapability {
            name: "code_review".to_string(),
            description: Some("Reviews code".to_string()),
            input_schema: None,
            output_schema: None,
        }]),
        status: Some(AgentStatus::Online),
        accepting_tasks: Some(true),
        metadata: Some(json!({"load": 0.5})),
    };

    let json_str = serde_json::to_string(&broadcast).expect("Must serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("Must be valid JSON");

    assert_eq!(parsed["agent_id"], "agent-123");
    assert_eq!(parsed["broadcast_type"], "capability");
}

/// Contract: DiscoverFeatures query/response
#[test]
fn contract_discover_features_schema() {
    let query = DiscoverFeaturesQuery {
        queries: Some(vec![
            FeatureQuery {
                feature_type: FeatureType::Protocol,
                match_pattern: Some("didcomm*".to_string()),
            },
            FeatureQuery {
                feature_type: FeatureType::McpTool,
                match_pattern: None,
            },
        ]),
    };

    let json_str = serde_json::to_string(&query).expect("Must serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("Must be valid JSON");

    let queries = parsed["queries"].as_array().expect("queries must be array");
    assert_eq!(queries.len(), 2);

    // Feature type uses kebab-case
    assert_eq!(queries[0]["feature-type"], "protocol");
    assert_eq!(queries[1]["feature-type"], "mcp-tool");
}

/// Contract: Error handling - invalid JSON must not panic
#[test]
fn contract_invalid_json_handling() {
    let invalid_json = r#"{"agent_id": 123, "task_type": null}"#; // Wrong types

    let result: Result<TaskRequest, _> = serde_json::from_str(invalid_json);
    // Should fail gracefully, not panic
    assert!(
        result.is_err(),
        "Invalid JSON should be rejected with error, not panic"
    );
}

/// Contract: Missing required fields must fail deserialization
#[test]
fn contract_missing_required_fields() {
    // AgentCard missing required 'url' field
    let incomplete = r#"{"name": "Test", "version": "1.0"}"#;

    let result: Result<AgentCard, _> = serde_json::from_str(incomplete);
    assert!(
        result.is_err(),
        "Missing required fields should cause deserialization error"
    );
}

/// Contract: Unknown fields should be ignored (forward compatibility)
#[test]
fn contract_forward_compatibility() {
    // JSON with fields that don't exist in current struct
    let extended_json = r#"{
        "agent_id": "agent-123",
        "task_type": "test",
        "unknown_field": "should be ignored",
        "another_unknown": 42
    }"#;

    let result: Result<TaskRequest, _> = serde_json::from_str(extended_json);
    assert!(
        result.is_ok(),
        "Unknown fields should be ignored for forward compatibility"
    );

    let request = result.unwrap();
    assert_eq!(request.agent_id, "agent-123");
    assert_eq!(request.task_type, "test");
}

/// Contract: ChatOpenRequest with authentication token
#[test]
fn contract_chat_open_request_schema() {
    let request = ChatOpenRequest {
        context: Some(json!({"previous_messages": []})),
        metadata: Some(json!({"client": "test"})),
        token: Some("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9".to_string()),
    };

    let json_str = serde_json::to_string(&request).expect("Must serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("Must be valid JSON");

    assert!(parsed.get("context").is_some());
    assert!(parsed.get("token").is_some());

    // Optional fields with skip_serializing_if should be omitted when None
    let minimal = ChatOpenRequest {
        context: None,
        metadata: None,
        token: None,
    };
    let minimal_json = serde_json::to_string(&minimal).expect("Must serialize");
    assert!(
        !minimal_json.contains("context"),
        "None values should be skipped during serialization"
    );
}

/// Contract: MetricsAck serialization
#[test]
fn contract_metrics_ack_schema() {
    let ack = MetricsAck {
        session_id: "session-123".to_string(),
        last_seq: 42,
        client_metrics: Some(ClientMetrics {
            buffer_size: 10,
            processing_latency_ms: Some(100),
            ready_for_more: true,
        }),
    };

    let json_str = serde_json::to_string(&ack).expect("Must serialize");
    let parsed: serde_json::Value = serde_json::from_str(&json_str).expect("Must be valid JSON");

    assert_eq!(parsed["session_id"], "session-123");
    assert_eq!(parsed["last_seq"], 42);
    assert!(parsed.get("client_metrics").is_some());
}

/// Contract: Complex nested structure round-trip
#[test]
fn contract_complex_nested_roundtrip() {
    let original = MessageSendRequest {
        message: Message {
            parts: vec![
                MessagePart::Text {
                    content: "Hello world".to_string(),
                },
                MessagePart::File {
                    name: "test.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    data: "SGVsbG8gV29ybGQ=".to_string(),
                    is_uri: false,
                },
            ],
            metadata: Some(json!({"importance": "high"})),
        },
        task_id: Some("task-456".to_string()),
    };

    let json_str = serde_json::to_string(&original).expect("Must serialize");
    let restored: MessageSendRequest = serde_json::from_str(&json_str).expect("Must deserialize");

    assert_eq!(original.message.parts.len(), restored.message.parts.len());
    assert_eq!(original.task_id, restored.task_id);
}
