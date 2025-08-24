use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};

/// JSON-RPC 2.0 request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
    pub id: Option<JsonRpcId>,
}

/// JSON-RPC 2.0 response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<JsonRpcId>,
}

/// JSON-RPC 2.0 notification (no id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// JSON-RPC 2.0 ID (can be number or string)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(u64),
    String(String),
}

/// JSON-RPC message that can be request, response, or notification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Response(JsonRpcResponse),
    Notification(JsonRpcNotification),
}

/// JSON-RPC client for bidirectional communication
pub struct JsonRpcClient {
    next_id: AtomicU64,
}

impl JsonRpcClient {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
        }
    }

    /// Create a new request with auto-incrementing ID
    pub fn create_request(&self, method: String, params: Option<Value>) -> JsonRpcRequest {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            method,
            params,
            id: Some(JsonRpcId::Number(id)),
        }
    }

    /// Create a notification (no ID, no response expected)
    pub fn create_notification(method: String, params: Option<Value>) -> JsonRpcNotification {
        JsonRpcNotification {
            jsonrpc: "2.0".to_string(),
            method,
            params,
        }
    }

    /// Create a successful response
    pub fn create_response(id: Option<JsonRpcId>, result: Value) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: Some(result),
            error: None,
            id,
        }
    }

    /// Create an error response
    pub fn create_error_response(
        id: Option<JsonRpcId>,
        code: i32,
        message: String,
        data: Option<Value>,
    ) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data,
            }),
            id,
        }
    }
}

impl Default for JsonRpcClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Standard JSON-RPC error codes
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;

    // Custom error codes for Claude Code
    pub const NODE_PROCESS_ERROR: i32 = -32000;
    pub const SDK_ERROR: i32 = -32001;
    pub const POLICY_VIOLATION: i32 = -32002;
    pub const BUDGET_EXCEEDED: i32 = -32003;
    pub const SESSION_ERROR: i32 = -32004;
}
