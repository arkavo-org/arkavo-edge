//! Core types and traits for Model Context Protocol (MCP)
//!
//! This crate provides the fundamental types and traits needed for MCP
//! without any runtime dependencies.

pub mod code_scanner;
pub mod integrity;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Schema definition for an MCP tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// The name of the tool
    pub name: String,
    /// Alternative names for the tool (for backwards compatibility)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    /// Human-readable description of what the tool does
    pub description: String,
    /// JSON Schema for the tool's parameters
    pub parameters: Value,
}

/// Core trait that all MCP tools must implement
#[async_trait]
pub trait Tool: Send + Sync {
    /// Execute the tool with the given parameters
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>>;

    /// Get the schema for this tool
    fn schema(&self) -> &ToolSchema;
}

/// Request to execute a tool
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolRequest {
    /// Name of the tool to execute
    pub tool_name: String,
    /// Parameters for the tool
    pub params: Value,
}

/// Response from executing a tool
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolResponse {
    /// Name of the tool that was executed
    pub tool_name: String,
    /// Result of the execution
    pub result: Value,
    /// Whether the execution was successful
    pub success: bool,
    /// MCP-I identity proof and metadata
    #[serde(rename = "_meta", skip_serializing_if = "Option::is_none", default)]
    pub meta: Option<Value>,
}

/// JSON-RPC request structure
#[derive(Debug, Serialize, Deserialize)]
pub struct RpcRequest {
    /// JSON-RPC version (should be "2.0")
    pub jsonrpc: String,
    /// Request ID
    pub id: Option<Value>,
    /// Method name
    pub method: String,
    /// Method parameters
    pub params: Option<Value>,
}

/// JSON-RPC response structure
#[derive(Debug, Serialize, Deserialize)]
pub struct RpcResponse {
    /// JSON-RPC version (should be "2.0")
    pub jsonrpc: String,
    /// Request ID (should match the request)
    pub id: Option<Value>,
    /// Result of the method call
    pub result: Option<Value>,
    /// Error information if the call failed
    pub error: Option<RpcError>,
}

/// JSON-RPC error structure
#[derive(Debug, Serialize, Deserialize)]
pub struct RpcError {
    /// Error code
    pub code: i32,
    /// Error message
    pub message: String,
    /// Additional error data
    pub data: Option<Value>,
}

/// JSON-RPC notification structure (request without id)
#[derive(Debug, Serialize, Deserialize)]
pub struct RpcNotification {
    /// JSON-RPC version (should be "2.0")
    pub jsonrpc: String,
    /// Method name
    pub method: String,
    /// Method parameters
    pub params: Option<Value>,
}

/// MCP Tool definition for remote tool discovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    /// Tool name
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// JSON Schema for input parameters
    #[serde(rename = "inputSchema", skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
}

/// Trait for MCP client abstraction
///
/// This trait provides a unified interface for communicating with MCP servers,
/// whether via subprocess, HTTP, or other transports.
pub trait McpClient: Send + Sync {
    /// List all tools available from the MCP server
    fn list_tools(&self) -> Result<Vec<McpTool>, Box<dyn std::error::Error + Send + Sync>>;

    /// Call a tool on the MCP server
    ///
    /// # Arguments
    /// * `tool_name` - Name of the tool to call
    /// * `args` - JSON arguments for the tool
    /// * `llm_origin` - Identifier for the LLM making the call (for logging/auditing)
    fn call_tool(
        &self,
        tool_name: &str,
        args: Value,
        llm_origin: &str,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>>;
}

/// Standard JSON-RPC error codes
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)] // tokio::test uses block_on internally

    use super::*;
    use arkavo_test_macros::spec;
    use serde_json::json;

    struct EchoParamsTool {
        schema: ToolSchema,
    }

    impl EchoParamsTool {
        fn new() -> Self {
            Self {
                schema: ToolSchema {
                    name: "echo_params".to_string(),
                    aliases: None,
                    description: "Echoes the params it receives".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {
                            "text": { "type": "string" }
                        }
                    }),
                },
            }
        }
    }

    #[async_trait]
    impl Tool for EchoParamsTool {
        async fn execute(
            &self,
            params: Value,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            Ok(json!({ "echo": params }))
        }

        fn schema(&self) -> &ToolSchema {
            &self.schema
        }
    }

    #[spec("MCPD-001")]
    #[tokio::test]
    async fn tool_trait_execute_and_schema() {
        let tool = EchoParamsTool::new();

        let schema = tool.schema();
        assert_eq!(schema.name, "echo_params");
        assert!(schema.parameters.get("properties").is_some());

        let result = tool
            .execute(json!({ "text": "hi" }))
            .await
            .expect("execute should succeed");
        assert_eq!(result, json!({ "echo": { "text": "hi" } }));
    }

    #[spec("MCPD-001")]
    #[test]
    fn tool_is_send_sync_and_object_safe() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<EchoParamsTool>();
        assert_send_sync::<Box<dyn Tool>>();

        // A schema with no parameters is still a valid schema.
        let schema = ToolSchema {
            name: "no_params".to_string(),
            aliases: None,
            description: "Takes no parameters".to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
        };
        let roundtrip: ToolSchema =
            serde_json::from_str(&serde_json::to_string(&schema).unwrap()).unwrap();
        assert_eq!(roundtrip.name, "no_params");
        assert_eq!(roundtrip.parameters["type"], "object");
    }

    #[spec("MCPD-002")]
    #[test]
    fn rpc_request_response_cycle_preserves_id_and_version() {
        let request: RpcRequest = serde_json::from_value(json!({
            "jsonrpc": "2.0",
            "id": 42,
            "method": "execute_tool",
            "params": { "tool_name": "echo_params", "params": {} }
        }))
        .unwrap();
        assert_eq!(request.method, "execute_tool");

        let response = RpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: Some(json!({ "ok": true })),
            error: None,
        };
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["jsonrpc"], "2.0");
        assert_eq!(wire["id"], 42);
        assert!(wire["error"].is_null());
    }

    #[spec("MCPD-002")]
    #[test]
    fn rpc_error_codes_follow_jsonrpc_standard() {
        assert_eq!(error_codes::PARSE_ERROR, -32700);
        assert_eq!(error_codes::INVALID_REQUEST, -32600);
        assert_eq!(error_codes::METHOD_NOT_FOUND, -32601);
        assert_eq!(error_codes::INVALID_PARAMS, -32602);
        assert_eq!(error_codes::INTERNAL_ERROR, -32603);

        let response = RpcResponse {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(7)),
            result: None,
            error: Some(RpcError {
                code: error_codes::METHOD_NOT_FOUND,
                message: "Unknown method: bogus".to_string(),
                data: None,
            }),
        };
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["error"]["code"], -32601);
        assert_eq!(wire["id"], 7);
    }

    #[spec("MCPD-003")]
    #[test]
    fn tool_schema_roundtrip_preserves_aliases() {
        let schema = ToolSchema {
            name: "git_status".to_string(),
            aliases: Some(vec!["git_state".to_string(), "status".to_string()]),
            description: "Show the working tree".to_string(),
            parameters: json!({ "type": "object", "properties": {} }),
        };

        let wire = serde_json::to_value(&schema).unwrap();
        assert_eq!(wire["name"], "git_status");
        assert_eq!(wire["aliases"], json!(["git_state", "status"]));

        let parsed: ToolSchema = serde_json::from_value(wire).unwrap();
        assert_eq!(
            parsed.aliases.as_deref(),
            Some(&["git_state".to_string(), "status".to_string()][..])
        );
    }

    #[spec("MCPD-003")]
    #[test]
    fn tool_schema_without_aliases_omits_the_field() {
        let schema = ToolSchema {
            name: "plain".to_string(),
            aliases: None,
            description: "No aliases".to_string(),
            parameters: json!({ "type": "object" }),
        };

        let wire = serde_json::to_value(&schema).unwrap();
        assert!(wire.get("aliases").is_none());
        assert_ne!(wire["name"].as_str().unwrap(), "");
        assert!(wire["parameters"].is_object());
    }

    /// In-memory client used to exercise the McpClient trait contract; this
    /// crate defines the trait, so conformance is what can be tested here.
    struct StaticClient {
        tools: Vec<McpTool>,
    }

    impl McpClient for StaticClient {
        fn list_tools(&self) -> Result<Vec<McpTool>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(self.tools.clone())
        }

        fn call_tool(
            &self,
            tool_name: &str,
            args: Value,
            _llm_origin: &str,
        ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
            if self.tools.iter().any(|t| t.name == tool_name) {
                Ok(json!({ "called": tool_name, "args": args }))
            } else {
                Err(format!("unknown tool: {tool_name}").into())
            }
        }
    }

    fn static_client() -> Box<dyn McpClient> {
        Box::new(StaticClient {
            tools: vec![McpTool {
                name: "remote_echo".to_string(),
                description: "Echo remotely".to_string(),
                input_schema: Some(json!({ "type": "object" })),
            }],
        })
    }

    #[spec("MCPD-006")]
    #[test]
    fn mcp_client_list_tools_returns_names_and_schemas() {
        let client = static_client();

        let tools = client.list_tools().expect("list_tools should succeed");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "remote_echo");
        assert!(tools[0].input_schema.is_some());

        // McpTool serializes with the MCP "inputSchema" field name.
        let wire = serde_json::to_value(&tools[0]).unwrap();
        assert!(wire.get("inputSchema").is_some());
    }

    #[spec("MCPD-006")]
    #[test]
    fn mcp_client_call_tool_invokes_discovered_tools() {
        let client = static_client();

        let result = client
            .call_tool("remote_echo", json!({ "text": "hi" }), "test")
            .expect("call_tool should succeed");
        assert_eq!(result["called"], "remote_echo");

        let err = client
            .call_tool("not_registered", json!({}), "test")
            .expect_err("unknown tool must fail");
        assert!(err.to_string().contains("not_registered"));
    }
}
