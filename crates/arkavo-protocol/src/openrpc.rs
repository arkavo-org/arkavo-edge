use crate::types::{
    AgentDiscoverFilter, DiscoveredAgent, PromiseCapability, PromiseDeclareResponse,
    PromiseRequest, PromiseResponse, PromiseStatus,
};
use schemars::{JsonSchema, schema_for};
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
            OpenRpcMethod {
                name: "promise_request".to_string(),
                description: Some("Request a promise from another agent".to_string()),
                summary: Some("Promise request".to_string()),
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
                        name: "promise_type".to_string(),
                        description: Some("The type of promise being requested".to_string()),
                        required: Some(true),
                        schema: json!({
                            "type": "string"
                        }),
                    },
                    OpenRpcParam {
                        name: "payload".to_string(),
                        description: Some("Additional data for the promise request".to_string()),
                        required: Some(false),
                        schema: json!({
                            "type": "object"
                        }),
                    },
                ],
                result: OpenRpcResult {
                    name: "promise_response".to_string(),
                    description: Some("The promise response from the agent".to_string()),
                    schema: serde_json::to_value(schema_for!(PromiseResponse)).unwrap(),
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
                    name: Some("Simple promise request".to_string()),
                    description: None,
                    params: vec![
                        OpenRpcExampleParam {
                            name: "agent_id".to_string(),
                            value: json!("550e8400-e29b-41d4-a716-446655440000"),
                        },
                        OpenRpcExampleParam {
                            name: "promise_type".to_string(),
                            value: json!("data_access"),
                        },
                    ],
                    result: Some(json!({
                        "promise_id": "660e8400-e29b-41d4-a716-446655440001",
                        "status": "accepted",
                        "data": {}
                    })),
                }]),
            },
            OpenRpcMethod {
                name: "promise_declare".to_string(),
                description: Some("Declare a promise capability to other agents".to_string()),
                summary: Some("Promise declaration".to_string()),
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
                        name: "promises".to_string(),
                        description: Some("List of promises the agent can fulfill".to_string()),
                        required: Some(true),
                        schema: serde_json::to_value(schema_for!(Vec<PromiseCapability>)).unwrap(),
                    },
                ],
                result: OpenRpcResult {
                    name: "declaration_result".to_string(),
                    description: Some("Confirmation of promise declaration".to_string()),
                    schema: serde_json::to_value(schema_for!(PromiseDeclareResponse)).unwrap(),
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
        ],
        components: Some(OpenRpcComponents {
            schemas: Some({
                let mut schemas = HashMap::new();

                schemas.insert(
                    "PromiseStatus".to_string(),
                    serde_json::to_value(schema_for!(PromiseStatus)).unwrap(),
                );

                schemas.insert(
                    "PromiseRequest".to_string(),
                    serde_json::to_value(schema_for!(PromiseRequest)).unwrap(),
                );

                schemas.insert(
                    "PromiseResponse".to_string(),
                    serde_json::to_value(schema_for!(PromiseResponse)).unwrap(),
                );

                schemas.insert(
                    "PromiseCapability".to_string(),
                    serde_json::to_value(schema_for!(PromiseCapability)).unwrap(),
                );

                schemas.insert(
                    "PromiseDeclareResponse".to_string(),
                    serde_json::to_value(schema_for!(PromiseDeclareResponse)).unwrap(),
                );

                schemas.insert(
                    "AgentDiscoverFilter".to_string(),
                    serde_json::to_value(schema_for!(AgentDiscoverFilter)).unwrap(),
                );

                schemas.insert(
                    "DiscoveredAgent".to_string(),
                    serde_json::to_value(schema_for!(DiscoveredAgent)).unwrap(),
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
        assert!(json.contains("promise_request"));
        assert!(json.contains("promise_declare"));
        assert!(json.contains("agent_discover"));
    }

    #[test]
    fn test_schema_has_components() {
        let schema = generate_openrpc_schema();
        assert!(schema.components.is_some());
        let components = schema.components.unwrap();
        assert!(components.schemas.is_some());
        let schemas = components.schemas.unwrap();
        assert!(schemas.contains_key("PromiseStatus"));
        assert!(schemas.contains_key("PromiseRequest"));
        assert!(schemas.contains_key("PromiseResponse"));
    }
}
