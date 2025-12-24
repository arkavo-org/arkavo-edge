//! Core types and traits for Model Context Protocol (MCP)
//!
//! This crate provides the fundamental types and traits needed for MCP
//! without any runtime dependencies.

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

/// Standard JSON-RPC error codes
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
}
