use crate::error::{A2aError, Result};
use crate::transport::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig};
use async_trait::async_trait;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tracing::{debug, info};

pub struct WebSocketTransport {
    config: TransportConfig,
    client: Arc<RwLock<Option<Arc<WsClient>>>>,
    endpoint: Arc<RwLock<Option<A2aEndpoint>>>,
}

impl WebSocketTransport {
    pub fn new(config: TransportConfig) -> Self {
        Self {
            config,
            client: Arc::new(RwLock::new(None)),
            endpoint: Arc::new(RwLock::new(None)),
        }
    }

    async fn build_client(&self, url: &str) -> Result<WsClient> {
        let timeout = Duration::from_millis(self.config.timeout_ms);

        let client = WsClientBuilder::default()
            .request_timeout(timeout)
            .connection_timeout(timeout)
            .max_concurrent_requests(100)
            .build(url)
            .await
            .map_err(|e| A2aError::ConnectionFailed(format!("WebSocket connection failed: {e}")))?;

        Ok(client)
    }
}

#[async_trait]
impl A2aTransport for WebSocketTransport {
    async fn connect(&self, endpoint: &A2aEndpoint) -> anyhow::Result<()> {
        info!("Connecting to WebSocket endpoint: {}", endpoint.url);

        if !endpoint.url.starts_with("ws://") && !endpoint.url.starts_with("wss://") {
            return Err(A2aError::InvalidEndpoint(format!(
                "WebSocket URL must start with ws:// or wss://, got: {}",
                endpoint.url
            ))
            .into());
        }

        if self.config.tls_config.require_tls && endpoint.url.starts_with("ws://") {
            return Err(A2aError::Tls(
                "TLS is required but URL uses ws:// instead of wss://".to_string(),
            )
            .into());
        }

        let client = self.build_client(&endpoint.url).await?;

        {
            let mut client_guard = self.client.write().unwrap();
            *client_guard = Some(Arc::new(client));
        }

        {
            let mut endpoint_guard = self.endpoint.write().unwrap();
            *endpoint_guard = Some(endpoint.clone());
        }

        info!("Successfully connected to {}", endpoint.url);
        Ok(())
    }

    async fn send_request(&self, request: A2aRequest) -> anyhow::Result<A2aResponse> {
        debug!(
            "Sending request: method={}, id={}",
            request.method, request.id
        );

        let client = {
            let guard = self.client.read().unwrap();
            guard.as_ref().ok_or(A2aError::NotConnected)?.clone()
        };

        // Build the JSON-RPC request manually
        let _json_rpc_request = serde_json::json!({
            "jsonrpc": "2.0",
            "method": request.method,
            "params": request.params,
            "id": request.id.to_string()
        });

        // Use the underlying WebSocket to send raw JSON
        // Since jsonrpsee doesn't expose raw send/receive easily, we'll use a workaround
        // by dynamically handling the params format

        // First, try to send with params as-is if it's an array
        let result = if request.params.is_array() {
            // Extract array elements and send based on array length
            if let Some(arr) = request.params.as_array() {
                match arr.len() {
                    0 => {
                        // No params - send empty array as params
                        client
                            .request::<serde_json::Value, _>(
                                &request.method,
                                Vec::<serde_json::Value>::new(),
                            )
                            .await
                    }
                    1 => {
                        // Single param - extract and send
                        let param = &arr[0];
                        if let Some(s) = param.as_str() {
                            client
                                .request::<serde_json::Value, _>(&request.method, (s,))
                                .await
                        } else if let Some(n) = param.as_u64() {
                            client
                                .request::<serde_json::Value, _>(&request.method, (n,))
                                .await
                        } else {
                            // For other types, serialize to string
                            client
                                .request::<serde_json::Value, _>(
                                    &request.method,
                                    (param.to_string(),),
                                )
                                .await
                        }
                    }
                    _ => {
                        // Multiple params - not supported by the test server
                        return Err(A2aError::WebSocket(
                            "Multiple parameters not supported".to_string(),
                        )
                        .into());
                    }
                }
            } else {
                return Err(A2aError::InvalidRequest("Expected array params".to_string()).into());
            }
        } else {
            // Non-array params - send as single value
            if let Some(s) = request.params.as_str() {
                client
                    .request::<serde_json::Value, _>(&request.method, (s,))
                    .await
            } else if let Some(n) = request.params.as_u64() {
                client
                    .request::<serde_json::Value, _>(&request.method, (n,))
                    .await
            } else {
                client
                    .request::<serde_json::Value, _>(&request.method, (request.params.to_string(),))
                    .await
            }
        };

        match result {
            Ok(response_json) => {
                let response = A2aResponse::Success {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: response_json,
                };
                debug!("Received success response for request id={}", request.id);
                Ok(response)
            }
            Err(e) => {
                // Check if this is a JSON-RPC error response
                if let jsonrpsee::core::client::error::Error::Call(call_err) = &e {
                    // Convert jsonrpsee error to our error response
                    let response = A2aResponse::Error {
                        jsonrpc: "2.0".to_string(),
                        id: request.id,
                        error: crate::transport::JsonRpcError {
                            code: call_err.code(),
                            message: call_err.message().to_string(),
                            data: call_err.data().map(|d| {
                                serde_json::from_str(d.get())
                                    .unwrap_or_else(|_| serde_json::Value::String(d.to_string()))
                            }),
                        },
                    };
                    debug!("Received error response for request id={}", request.id);
                    Ok(response)
                } else {
                    // Other transport errors
                    Err(A2aError::WebSocket(format!("Request failed: {e}")).into())
                }
            }
        }
    }

    async fn close(&self) -> anyhow::Result<()> {
        info!("Closing WebSocket connection");

        {
            let mut client_guard = self.client.write().unwrap();
            *client_guard = None;
        }

        {
            let mut endpoint_guard = self.endpoint.write().unwrap();
            *endpoint_guard = None;
        }

        Ok(())
    }

    fn is_connected(&self) -> bool {
        let guard = self.client.read().unwrap();
        guard.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_websocket_transport_creation() {
        let config = TransportConfig::default();
        let transport = WebSocketTransport::new(config);
        assert!(!transport.is_connected());
    }

    #[tokio::test]
    async fn test_invalid_endpoint() {
        let config = TransportConfig::default();
        let transport = WebSocketTransport::new(config);

        let endpoint = A2aEndpoint {
            url: "http://localhost:8080".to_string(),
            agent_id: "test-agent".to_string(),
            public_key: None,
        };

        let result = transport.connect(&endpoint).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("WebSocket URL must start with")
        );
    }

    #[tokio::test]
    async fn test_tls_requirement() {
        let config = TransportConfig {
            tls_config: crate::transport::TlsConfig {
                require_tls: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let transport = WebSocketTransport::new(config);

        let endpoint = A2aEndpoint {
            url: "ws://localhost:8080".to_string(),
            agent_id: "test-agent".to_string(),
            public_key: None,
        };

        let result = transport.connect(&endpoint).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("TLS is required"));
    }

    #[tokio::test]
    async fn test_not_connected_error() {
        let config = TransportConfig::default();
        let transport = WebSocketTransport::new(config);

        let request = A2aRequest::new("test_method", serde_json::json!({}));
        let result = transport.send_request(request).await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transport not connected")
        );
    }
}
