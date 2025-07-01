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
            .map_err(|e| {
                A2aError::ConnectionFailed(format!("WebSocket connection failed: {e}"))
            })?;

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

        let params = vec![request.params];

        let response_json: serde_json::Value = client
            .request(&request.method, params)
            .await
            .map_err(|e| A2aError::WebSocket(format!("Request failed: {e}")))?;

        let response = A2aResponse::Success {
            jsonrpc: "2.0".to_string(),
            id: request.id,
            result: response_json,
        };

        debug!("Received response for request id={}", request.id);
        Ok(response)
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
