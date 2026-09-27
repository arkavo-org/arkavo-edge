use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aRequest {
    pub jsonrpc: String,
    pub id: Uuid,
    pub method: String,
    pub params: serde_json::Value,
}

impl A2aRequest {
    pub fn new(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: Uuid::new_v4(),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum A2aResponse {
    Success {
        jsonrpc: String,
        id: Uuid,
        result: serde_json::Value,
    },
    Error {
        jsonrpc: String,
        id: Uuid,
        error: JsonRpcError,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aEndpoint {
    pub url: String,
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

#[async_trait]
pub trait A2aTransport: Send + Sync {
    async fn connect(&self, endpoint: &A2aEndpoint) -> Result<()>;

    async fn send_request(&self, request: A2aRequest) -> Result<A2aResponse>;

    async fn close(&self) -> Result<()>;

    fn is_connected(&self) -> bool;
}

pub type A2aTransportRef = Arc<dyn A2aTransport>;

#[derive(Debug, Clone)]
pub struct TransportConfig {
    pub timeout_ms: u64,
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub tls_config: TlsConfig,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            timeout_ms: 30000,
            max_retries: 3,
            retry_delay_ms: 1000,
            tls_config: TlsConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TlsConfig {
    /// Certificate verification is always on. `false` is ignored.
    pub verify_cert: bool,
    pub require_tls: bool,
    pub client_cert_path: Option<String>,
    pub client_key_path: Option<String>,
    pub ca_cert_path: Option<String>,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            verify_cert: true,
            require_tls: true,
            client_cert_path: None,
            client_key_path: None,
            ca_cert_path: None,
        }
    }
}

pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    pub const SERVER_ERROR_START: i32 = -32000;
    pub const SERVER_ERROR_END: i32 = -32099;

    pub const TRANSPORT_ERROR: i32 = -32000;
    pub const CONNECTION_ERROR: i32 = -32001;
    pub const TIMEOUT_ERROR: i32 = -32002;
    pub const TLS_ERROR: i32 = -32003;
    pub const AUTHENTICATION_ERROR: i32 = -32004;
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    #[test]
    fn test_request_creation() {
        let request = A2aRequest::new("test_method", serde_json::json!({"key": "value"}));
        assert_eq!(request.jsonrpc, "2.0");
        assert_eq!(request.method, "test_method");
        assert!(!request.id.to_string().is_empty());
    }

    #[test]
    fn test_response_serialization() {
        let response = A2aResponse::Success {
            jsonrpc: "2.0".to_string(),
            id: Uuid::new_v4(),
            result: serde_json::json!({"status": "ok"}),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("jsonrpc"));
        assert!(json.contains("result"));
    }

    #[test]
    fn test_error_response() {
        let error_response = A2aResponse::Error {
            jsonrpc: "2.0".to_string(),
            id: Uuid::new_v4(),
            error: JsonRpcError {
                code: error_codes::METHOD_NOT_FOUND,
                message: "Method not found".to_string(),
                data: None,
            },
        };

        let json = serde_json::to_string(&error_response).unwrap();
        assert!(json.contains("error"));
        assert!(json.contains("-32601"));
    }
}
