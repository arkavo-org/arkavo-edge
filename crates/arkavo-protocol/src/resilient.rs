use crate::error::{A2aError, Result};
use crate::http::HttpTransport;
use crate::transport::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig};
use crate::websocket::WebSocketTransport;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Transport type indicator
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    WebSocket,
    Http,
}

/// Resilient transport that tries WebSocket first, falls back to HTTP
///
/// This transport provides automatic failover:
/// 1. Attempts WebSocket connection first (persistent, bidirectional)
/// 2. Falls back to HTTP if WebSocket fails (reliable, stateless)
/// 3. Periodically retries WebSocket when operating in HTTP fallback mode
pub struct ResilientTransport {
    #[allow(dead_code)]
    config: TransportConfig,
    ws_transport: WebSocketTransport,
    http_transport: HttpTransport,
    active_transport: Arc<RwLock<Option<TransportType>>>,
    endpoint: Arc<RwLock<Option<A2aEndpoint>>>,
    last_ws_attempt: Arc<RwLock<Instant>>,
    ws_retry_interval: Duration,
}

impl ResilientTransport {
    /// Create a new resilient transport
    pub fn new(config: TransportConfig) -> Result<Self> {
        let ws_transport = WebSocketTransport::new(config.clone());
        let http_transport = HttpTransport::new(config.clone())?;

        Ok(Self {
            config,
            ws_transport,
            http_transport,
            active_transport: Arc::new(RwLock::new(None)),
            endpoint: Arc::new(RwLock::new(None)),
            last_ws_attempt: Arc::new(RwLock::new(Instant::now())),
            ws_retry_interval: Duration::from_secs(30),
        })
    }

    /// Create with custom WebSocket retry interval
    pub fn with_retry_interval(config: TransportConfig, retry_interval: Duration) -> Result<Self> {
        let mut transport = Self::new(config)?;
        transport.ws_retry_interval = retry_interval;
        Ok(transport)
    }

    /// Get the currently active transport type
    pub async fn active_transport_type(&self) -> Option<TransportType> {
        *self.active_transport.read().await
    }

    /// Convert HTTP URL to WebSocket URL
    fn http_to_ws_url(url: &str) -> String {
        let ws_url = if url.starts_with("https://") {
            url.replace("https://", "wss://")
        } else if url.starts_with("http://") {
            url.replace("http://", "ws://")
        } else {
            format!("ws://{url}")
        };

        // Add /ws path if not present
        if ws_url.ends_with("/ws") {
            ws_url
        } else {
            format!("{}/ws", ws_url.trim_end_matches('/'))
        }
    }

    /// Try to connect via WebSocket
    async fn try_websocket_connect(&self, endpoint: &A2aEndpoint) -> bool {
        let ws_url = Self::http_to_ws_url(&endpoint.url);
        let ws_endpoint = A2aEndpoint {
            url: ws_url.clone(),
            agent_id: endpoint.agent_id.clone(),
            public_key: endpoint.public_key.clone(),
        };

        debug!("Attempting WebSocket connection to {}", ws_url);
        *self.last_ws_attempt.write().await = Instant::now();

        match self.ws_transport.connect(&ws_endpoint).await {
            Ok(()) => {
                info!("WebSocket connection established to {}", ws_url);
                true
            }
            Err(e) => {
                debug!("WebSocket connection failed: {}", e);
                false
            }
        }
    }

    /// Try to connect via HTTP
    async fn try_http_connect(&self, endpoint: &A2aEndpoint) -> Result<()> {
        debug!("Attempting HTTP connection to {}", endpoint.url);
        self.http_transport
            .connect(endpoint)
            .await
            .map_err(|e| A2aError::ConnectionFailed(format!("HTTP connection failed: {e}")))?;
        info!("HTTP connection established to {}", endpoint.url);
        Ok(())
    }

    /// Check if we should retry WebSocket connection
    async fn should_retry_websocket(&self) -> bool {
        let last_attempt = *self.last_ws_attempt.read().await;
        last_attempt.elapsed() >= self.ws_retry_interval
    }

    /// Attempt to upgrade from HTTP to WebSocket
    async fn try_upgrade_to_websocket(&self) -> bool {
        let endpoint = {
            let guard = self.endpoint.read().await;
            match guard.as_ref() {
                Some(ep) => ep.clone(),
                None => return false,
            }
        };

        if self.try_websocket_connect(&endpoint).await {
            *self.active_transport.write().await = Some(TransportType::WebSocket);
            info!("Upgraded from HTTP to WebSocket");
            true
        } else {
            false
        }
    }
}

#[async_trait]
impl A2aTransport for ResilientTransport {
    async fn connect(&self, endpoint: &A2aEndpoint) -> anyhow::Result<()> {
        info!(
            "ResilientTransport connecting to {} (agent: {})",
            endpoint.url, endpoint.agent_id
        );

        // Store endpoint for potential reconnection
        *self.endpoint.write().await = Some(endpoint.clone());

        // Try WebSocket first (preferred: persistent, bidirectional)
        if self.try_websocket_connect(endpoint).await {
            *self.active_transport.write().await = Some(TransportType::WebSocket);
            return Ok(());
        }

        // Fall back to HTTP
        warn!("WebSocket unavailable, falling back to HTTP");
        self.try_http_connect(endpoint).await?;
        *self.active_transport.write().await = Some(TransportType::Http);

        Ok(())
    }

    async fn send_request(&self, request: A2aRequest) -> anyhow::Result<A2aResponse> {
        let active = *self.active_transport.read().await;

        match active {
            Some(TransportType::WebSocket) => {
                // Try WebSocket
                match self.ws_transport.send_request(request.clone()).await {
                    Ok(response) => return Ok(response),
                    Err(e) => {
                        warn!("WebSocket request failed: {}, falling back to HTTP", e);
                        // WebSocket failed, try HTTP fallback
                        let endpoint = self.endpoint.read().await.clone();
                        if let Some(ep) = endpoint {
                            if self.try_http_connect(&ep).await.is_ok() {
                                *self.active_transport.write().await = Some(TransportType::Http);
                                return self.http_transport.send_request(request).await;
                            }
                        }
                        return Err(e);
                    }
                }
            }
            Some(TransportType::Http) => {
                // Check if we should retry WebSocket
                if self.should_retry_websocket().await {
                    debug!("Attempting WebSocket upgrade...");
                    if self.try_upgrade_to_websocket().await {
                        // Successfully upgraded, use WebSocket
                        return self.ws_transport.send_request(request).await;
                    }
                }

                // Use HTTP
                self.http_transport.send_request(request).await
            }
            None => Err(A2aError::NotConnected.into()),
        }
    }

    async fn close(&self) -> anyhow::Result<()> {
        info!("ResilientTransport closing connections");

        let active = *self.active_transport.read().await;

        // Close active transport
        match active {
            Some(TransportType::WebSocket) => {
                let _ = self.ws_transport.close().await;
            }
            Some(TransportType::Http) => {
                let _ = self.http_transport.close().await;
            }
            None => {}
        }

        // Clear state
        *self.active_transport.write().await = None;
        *self.endpoint.write().await = None;

        Ok(())
    }

    fn is_connected(&self) -> bool {
        // Use try_read to avoid blocking
        if let Ok(guard) = self.active_transport.try_read() {
            guard.is_some()
        } else {
            false
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::transport::TlsConfig;

    fn test_config() -> TransportConfig {
        TransportConfig {
            timeout_ms: 5000,
            max_retries: 1,
            retry_delay_ms: 100,
            tls_config: TlsConfig {
                require_tls: false,
                verify_cert: false,
                ..Default::default()
            },
        }
    }

    #[test]
    fn test_http_to_ws_url_conversion() {
        assert_eq!(
            ResilientTransport::http_to_ws_url("http://localhost:8080"),
            "ws://localhost:8080/ws"
        );
        assert_eq!(
            ResilientTransport::http_to_ws_url("https://localhost:8080"),
            "wss://localhost:8080/ws"
        );
        assert_eq!(
            ResilientTransport::http_to_ws_url("http://localhost:8080/"),
            "ws://localhost:8080/ws"
        );
        assert_eq!(
            ResilientTransport::http_to_ws_url("http://localhost:8080/ws"),
            "ws://localhost:8080/ws"
        );
    }

    #[tokio::test]
    async fn test_resilient_transport_creation() {
        let config = test_config();
        let transport = ResilientTransport::new(config).unwrap();
        assert!(!transport.is_connected());
        assert!(transport.active_transport_type().await.is_none());
    }

    #[tokio::test]
    async fn test_not_connected_error() {
        let config = test_config();
        let transport = ResilientTransport::new(config).unwrap();

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

    #[tokio::test]
    async fn test_close_not_connected() {
        let config = test_config();
        let transport = ResilientTransport::new(config).unwrap();

        // Should not error when closing unconnected transport
        let result = transport.close().await;
        assert!(result.is_ok());
    }
}
