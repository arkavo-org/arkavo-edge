use crate::types::{
    AgentDiscoverFilter, Attachment, ChatCapabilities, ChatOpenRequest, ChatSession,
    DiscoveredAgent, Message, MessageDelta, MessageDeltaContent, MessagePart, MessageSendRequest,
    MessageSendResponse, StreamEndReason, TaskCancelRequest, TaskCancelResponse, TaskCapability,
    TaskDeclareResponse, TaskGetRequest, TaskGetResponse, TaskRequest, TaskResponse, TaskStatus,
    UserMessage,
};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcDocument {
    pub openrpc: String,
    pub info: OpenRpcInfo,
    pub methods: Vec<OpenRpcMethod>,
    pub components: Option<OpenRpcComponents>,
    pub servers: Option<Vec<OpenRpcServer>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcInfo {
    pub title: String,
    pub description: Option<String>,
    pub version: String,
    pub contact: Option<OpenRpcContact>,
    pub license: Option<OpenRpcLicense>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcContact {
    pub name: Option<String>,
    pub url: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcLicense {
    pub name: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcMethod {
    pub name: String,
    pub description: Option<String>,
    pub summary: Option<String>,
    pub params: Vec<OpenRpcParam>,
    pub result: OpenRpcResult,
    pub errors: Option<Vec<OpenRpcError>>,
    pub examples: Option<Vec<OpenRpcExample>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcParam {
    pub name: String,
    pub description: Option<String>,
    pub required: Option<bool>,
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcResult {
    pub name: String,
    pub description: Option<String>,
    pub schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcExample {
    pub name: Option<String>,
    pub description: Option<String>,
    pub params: Vec<OpenRpcExampleParam>,
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcExampleParam {
    pub name: String,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcComponents {
    pub schemas: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OpenRpcServer {
    pub name: String,
    pub url: String,
    pub summary: Option<String>,
    pub description: Option<String>,
}

/// Generate the OpenRPC schema for the A2A protocol
///
/// # Panics
///
/// Panics if schema generation fails for any of the types
pub fn generate_openrpc_schema() -> OpenRpcDocument {
    OpenRpcDocument {
        openrpc: "1.2.6".to_string(),
        info: OpenRpcInfo {
            title: "Arkavo A2A Transport Protocol".to_string(),
            description: Some(
                "Agent-to-Agent communication protocol based on JSON-RPC 2.0".to_string(),
            ),
            version: env!("CARGO_PKG_VERSION").to_string(),
            contact: Some(OpenRpcContact {
                name: Some("Arkavo Edge Contributors".to_string()),
                url: Some("https://github.com/arkavo-org/arkavo-edge".to_string()),
                email: None,
            }),
            license: Some(OpenRpcLicense {
                name: "Apache-2.0".to_string(),
                url: Some("https://www.apache.org/licenses/LICENSE-2.0".to_string()),
            }),
        },
        methods: vec![
            // Introspection
            OpenRpcMethod {
                name: "rpc.discover".to_string(),
                description: Some("Returns OpenRPC schema document".to_string()),
                summary: Some("OpenRPC discovery".to_string()),
                params: vec![],
                result: OpenRpcResult {
                    name: "schema".to_string(),
                    description: Some("OpenRPC schema document".to_string()),
                    schema: json!({
                        "type": "object"
                    }),
                },
                errors: None,
                examples: None,
            },
            OpenRpcMethod {
                name: "task_request".to_string(),
                description: Some("Request a task from another agent".to_string()),
                summary: Some("Task request".to_string()),
                params: vec![
                    OpenRpcParam {
                        name: "agent_id".to_string(),
                        description: Some("The ID of the requesting agent".to_string()),
                        required: Some(true),
                        schema: json!({
                            "type": "string"
                        }),
                    },
                    OpenRpcParam {
                        name: "task_type".to_string(),
                        description: Some("The type of task being requested".to_string()),
                        required: Some(true),
                        schema: json!({
                            "type": "string"
                        }),
                    },
                    OpenRpcParam {
                        name: "payload".to_string(),
                        description: Some("Additional data for the task request".to_string()),
                        required: Some(false),
                        schema: json!({
                            "type": "object"
                        }),
                    },
                ],
                result: OpenRpcResult {
                    name: "task_response".to_string(),
                    description: Some("The task response from the agent".to_string()),
                    schema: serde_json::to_value(schema_for!(TaskResponse)).unwrap(),
                },
                errors: Some(vec![
                    OpenRpcError {
                        code: -32601,
                        message: "Method not found".to_string(),
                        data: None,
                    },
                    OpenRpcError {
                        code: -32602,
                        message: "Invalid params".to_string(),
                        data: None,
                    },
                ]),
                examples: Some(vec![OpenRpcExample {
                    name: Some("Simple task request".to_string()),
                    description: None,
                    params: vec![
                        OpenRpcExampleParam {
                            name: "agent_id".to_string(),
                            value: json!("550e8400-e29b-41d4-a716-446655440000"),
                        },
                        OpenRpcExampleParam {
                            name: "task_type".to_string(),
                            value: json!("data_access"),
                        },
                    ],
                    result: Some(json!({
                        "task_id": "660e8400-e29b-41d4-a716-446655440001",
                        "status": "accepted",
                        "data": {}
                    })),
                }]),
            },
            OpenRpcMethod {
                name: "task_declare".to_string(),
                description: Some("Declare a task capability to other agents".to_string()),
                summary: Some("Task declaration".to_string()),
                params: vec![
                    OpenRpcParam {
                        name: "agent_id".to_string(),
                        description: Some("The ID of the declaring agent".to_string()),
                        required: Some(true),
                        schema: json!({
                            "type": "string"
                        }),
                    },
                    OpenRpcParam {
                        name: "tasks".to_string(),
                        description: Some("List of tasks the agent can fulfill".to_string()),
                        required: Some(true),
                        schema: serde_json::to_value(schema_for!(Vec<TaskCapability>)).unwrap(),
                    },
                ],
                result: OpenRpcResult {
                    name: "declaration_result".to_string(),
                    description: Some("Confirmation of task declaration".to_string()),
                    schema: serde_json::to_value(schema_for!(TaskDeclareResponse)).unwrap(),
                },
                errors: None,
                examples: None,
            },
            OpenRpcMethod {
                name: "agent_discover".to_string(),
                description: Some("Discover available agents in the network".to_string()),
                summary: Some("Agent discovery".to_string()),
                params: vec![OpenRpcParam {
                    name: "filter".to_string(),
                    description: Some("Optional filter criteria for agent discovery".to_string()),
                    required: Some(false),
                    schema: serde_json::to_value(schema_for!(Option<AgentDiscoverFilter>)).unwrap(),
                }],
                result: OpenRpcResult {
                    name: "discovered_agents".to_string(),
                    description: Some("List of discovered agents".to_string()),
                    schema: serde_json::to_value(schema_for!(Vec<DiscoveredAgent>)).unwrap(),
                },
                errors: None,
                examples: None,
            },
            // A2A Protocol Methods
            OpenRpcMethod {
                name: "message/send".to_string(),
                description: Some("Send a message synchronously to an agent".to_string()),
                summary: Some("Send message".to_string()),
                params: vec![OpenRpcParam {
                    name: "request".to_string(),
                    description: Some("Message send request".to_string()),
                    required: Some(true),
                    schema: serde_json::to_value(schema_for!(MessageSendRequest)).unwrap(),
                }],
                result: OpenRpcResult {
                    name: "message_response".to_string(),
                    description: Some("Response containing task ID and status".to_string()),
                    schema: serde_json::to_value(schema_for!(MessageSendResponse)).unwrap(),
                },
                errors: None,
                examples: None,
            },
            OpenRpcMethod {
                name: "tasks/get".to_string(),
                description: Some("Get task status and result".to_string()),
                summary: Some("Get task".to_string()),
                params: vec![OpenRpcParam {
                    name: "request".to_string(),
                    description: Some("Task get request".to_string()),
                    required: Some(true),
                    schema: serde_json::to_value(schema_for!(TaskGetRequest)).unwrap(),
                }],
                result: OpenRpcResult {
                    name: "task_status".to_string(),
                    description: Some("Current task status and result".to_string()),
                    schema: serde_json::to_value(schema_for!(TaskGetResponse)).unwrap(),
                },
                errors: None,
                examples: None,
            },
            OpenRpcMethod {
                name: "tasks/cancel".to_string(),
                description: Some("Cancel a running task".to_string()),
                summary: Some("Cancel task".to_string()),
                params: vec![OpenRpcParam {
                    name: "request".to_string(),
                    description: Some("Task cancel request".to_string()),
                    required: Some(true),
                    schema: serde_json::to_value(schema_for!(TaskCancelRequest)).unwrap(),
                }],
                result: OpenRpcResult {
                    name: "cancel_response".to_string(),
                    description: Some("Task cancellation result".to_string()),
                    schema: serde_json::to_value(schema_for!(TaskCancelResponse)).unwrap(),
                },
                errors: None,
                examples: None,
            },
            // Chat Protocol Methods
            OpenRpcMethod {
                name: "chat_open".to_string(),
                description: Some("Open a new chat session with the agent".to_string()),
                summary: Some("Open chat session".to_string()),
                params: vec![OpenRpcParam {
                    name: "request".to_string(),
                    description: Some(
                        "Chat open request with optional context and metadata".to_string(),
                    ),
                    required: Some(true),
                    schema: serde_json::to_value(schema_for!(ChatOpenRequest)).unwrap(),
                }],
                result: OpenRpcResult {
                    name: "session".to_string(),
                    description: Some("Chat session with session ID and capabilities".to_string()),
                    schema: serde_json::to_value(schema_for!(ChatSession)).unwrap(),
                },
                errors: Some(vec![
                    OpenRpcError {
                        code: -32004,
                        message: "Authentication failed".to_string(),
                        data: None,
                    },
                    OpenRpcError {
                        code: -32003,
                        message: "Rate limit exceeded".to_string(),
                        data: None,
                    },
                ]),
                examples: Some(vec![OpenRpcExample {
                    name: Some("Open new chat session".to_string()),
                    description: None,
                    params: vec![OpenRpcExampleParam {
                        name: "request".to_string(),
                        value: json!({
                            "context": {"user_id": "user123"},
                            "metadata": {"source": "mobile_app"}
                        }),
                    }],
                    result: Some(json!({
                        "session_id": "sess_abc123",
                        "created_at": "2025-01-16T10:00:00Z",
                        "capabilities": {
                            "supports_attachments": false,
                            "supports_tools": true
                        }
                    })),
                }]),
            },
            OpenRpcMethod {
                name: "chat_send".to_string(),
                description: Some("Send a message within an existing chat session".to_string()),
                summary: Some("Send chat message".to_string()),
                params: vec![
                    OpenRpcParam {
                        name: "session_id".to_string(),
                        description: Some("The session ID from chat_open".to_string()),
                        required: Some(true),
                        schema: json!({
                            "type": "string"
                        }),
                    },
                    OpenRpcParam {
                        name: "message".to_string(),
                        description: Some(
                            "User message with content and optional attachments".to_string(),
                        ),
                        required: Some(true),
                        schema: serde_json::to_value(schema_for!(UserMessage)).unwrap(),
                    },
                ],
                result: OpenRpcResult {
                    name: "result".to_string(),
                    description: Some("Empty result on success".to_string()),
                    schema: json!({
                        "type": "null"
                    }),
                },
                errors: Some(vec![
                    OpenRpcError {
                        code: -32001,
                        message: "Session not found".to_string(),
                        data: None,
                    },
                    OpenRpcError {
                        code: -32002,
                        message: "Buffer full".to_string(),
                        data: None,
                    },
                    OpenRpcError {
                        code: -32003,
                        message: "Rate limit exceeded".to_string(),
                        data: None,
                    },
                ]),
                examples: None,
            },
            OpenRpcMethod {
                name: "chat_close".to_string(),
                description: Some("Close an existing chat session".to_string()),
                summary: Some("Close chat session".to_string()),
                params: vec![OpenRpcParam {
                    name: "session_id".to_string(),
                    description: Some("The session ID to close".to_string()),
                    required: Some(true),
                    schema: json!({
                        "type": "string"
                    }),
                }],
                result: OpenRpcResult {
                    name: "result".to_string(),
                    description: Some("Empty result on success".to_string()),
                    schema: json!({
                        "type": "null"
                    }),
                },
                errors: Some(vec![OpenRpcError {
                    code: -32001,
                    message: "Session not found".to_string(),
                    data: None,
                }]),
                examples: None,
            },
        ],
        components: Some(OpenRpcComponents {
            schemas: Some({
                let mut schemas = HashMap::new();

                schemas.insert(
                    "TaskStatus".to_string(),
                    serde_json::to_value(schema_for!(TaskStatus)).unwrap(),
                );

                schemas.insert(
                    "TaskRequest".to_string(),
                    serde_json::to_value(schema_for!(TaskRequest)).unwrap(),
                );

                schemas.insert(
                    "TaskResponse".to_string(),
                    serde_json::to_value(schema_for!(TaskResponse)).unwrap(),
                );

                schemas.insert(
                    "TaskCapability".to_string(),
                    serde_json::to_value(schema_for!(TaskCapability)).unwrap(),
                );

                schemas.insert(
                    "TaskDeclareResponse".to_string(),
                    serde_json::to_value(schema_for!(TaskDeclareResponse)).unwrap(),
                );

                schemas.insert(
                    "AgentDiscoverFilter".to_string(),
                    serde_json::to_value(schema_for!(AgentDiscoverFilter)).unwrap(),
                );

                schemas.insert(
                    "DiscoveredAgent".to_string(),
                    serde_json::to_value(schema_for!(DiscoveredAgent)).unwrap(),
                );

                // A2A Protocol Schemas
                schemas.insert(
                    "MessageSendRequest".to_string(),
                    serde_json::to_value(schema_for!(MessageSendRequest)).unwrap(),
                );

                schemas.insert(
                    "MessageSendResponse".to_string(),
                    serde_json::to_value(schema_for!(MessageSendResponse)).unwrap(),
                );

                schemas.insert(
                    "Message".to_string(),
                    serde_json::to_value(schema_for!(Message)).unwrap(),
                );

                schemas.insert(
                    "MessagePart".to_string(),
                    serde_json::to_value(schema_for!(MessagePart)).unwrap(),
                );

                schemas.insert(
                    "TaskGetRequest".to_string(),
                    serde_json::to_value(schema_for!(TaskGetRequest)).unwrap(),
                );

                schemas.insert(
                    "TaskGetResponse".to_string(),
                    serde_json::to_value(schema_for!(TaskGetResponse)).unwrap(),
                );

                schemas.insert(
                    "TaskCancelRequest".to_string(),
                    serde_json::to_value(schema_for!(TaskCancelRequest)).unwrap(),
                );

                schemas.insert(
                    "TaskCancelResponse".to_string(),
                    serde_json::to_value(schema_for!(TaskCancelResponse)).unwrap(),
                );

                // Chat Protocol Schemas
                schemas.insert(
                    "ChatOpenRequest".to_string(),
                    serde_json::to_value(schema_for!(ChatOpenRequest)).unwrap(),
                );

                schemas.insert(
                    "ChatSession".to_string(),
                    serde_json::to_value(schema_for!(ChatSession)).unwrap(),
                );

                schemas.insert(
                    "ChatCapabilities".to_string(),
                    serde_json::to_value(schema_for!(ChatCapabilities)).unwrap(),
                );

                schemas.insert(
                    "UserMessage".to_string(),
                    serde_json::to_value(schema_for!(UserMessage)).unwrap(),
                );

                schemas.insert(
                    "Attachment".to_string(),
                    serde_json::to_value(schema_for!(Attachment)).unwrap(),
                );

                schemas.insert(
                    "MessageDelta".to_string(),
                    serde_json::to_value(schema_for!(MessageDelta)).unwrap(),
                );

                schemas.insert(
                    "MessageDeltaContent".to_string(),
                    serde_json::to_value(schema_for!(MessageDeltaContent)).unwrap(),
                );

                schemas.insert(
                    "StreamEndReason".to_string(),
                    serde_json::to_value(schema_for!(StreamEndReason)).unwrap(),
                );

                schemas
            }),
        }),
        servers: Some(vec![
            OpenRpcServer {
                name: "Local Development".to_string(),
                url: "http://localhost:8765".to_string(),
                summary: Some("Local development server".to_string()),
                description: Some("Default local development server for testing".to_string()),
            },
            OpenRpcServer {
                name: "Production".to_string(),
                url: "https://api.arkavo.io/a2a".to_string(),
                summary: Some("Production server".to_string()),
                description: Some("Production A2A endpoint (example)".to_string()),
            },
        ]),
    }
}

/// Convert the OpenRPC document to JSON string
pub fn openrpc_to_json() -> Result<String, serde_json::Error> {
    let schema = generate_openrpc_schema();
    serde_json::to_string_pretty(&schema)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn test_openrpc_generation() {
        let schema = generate_openrpc_schema();
        assert_eq!(schema.openrpc, "1.2.6");
        assert_eq!(schema.info.title, "Arkavo A2A Transport Protocol");
        assert!(!schema.methods.is_empty());
    }

    #[test]
    fn test_openrpc_json_serialization() {
        let json = openrpc_to_json().unwrap();
        assert!(json.contains("\"openrpc\": \"1.2.6\""));
        assert!(json.contains("task_request"));
        assert!(json.contains("task_declare"));
        assert!(json.contains("agent_discover"));
        assert!(json.contains("chat_open"));
        assert!(json.contains("chat_send"));
        assert!(json.contains("chat_close"));
    }

    #[test]
    fn test_schema_has_components() {
        let schema = generate_openrpc_schema();
        assert!(schema.components.is_some());
        let components = schema.components.unwrap();
        assert!(components.schemas.is_some());
        let schemas = components.schemas.unwrap();
        assert!(schemas.contains_key("TaskStatus"));
        assert!(schemas.contains_key("TaskRequest"));
        assert!(schemas.contains_key("TaskResponse"));
        assert!(schemas.contains_key("ChatOpenRequest"));
        assert!(schemas.contains_key("ChatSession"));
        assert!(schemas.contains_key("UserMessage"));
        assert!(schemas.contains_key("MessageDelta"));
    }
}
