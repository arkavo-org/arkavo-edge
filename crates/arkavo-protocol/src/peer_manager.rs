//! Peer Manager for A2A Agent Communication
//!
//! Manages connections to peer agents for agent-to-agent communication.

// Pre-existing architectural patterns - lock held across await is intentional
// and the public API is simple enough that panics on poisoned locks are appropriate.
#![allow(clippy::future_not_send)]
#![allow(clippy::await_holding_lock)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::significant_drop_tightening)]

use crate::http::HttpTransport;
use crate::transport::{
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, A2aTransportRef, TransportConfig,
};
use crate::websocket::WebSocketTransport;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

/// Transport type for A2A communication
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    /// HTTP transport for stateless, transactional requests
    Http,
    /// WebSocket transport for stateful, streaming requests
    WebSocket,
    /// Externally-provided transport (e.g., OpenClaw WS adapter)
    Custom,
}

/// Configuration for peer manager
#[derive(Debug, Clone)]
pub struct PeerManagerConfig {
    /// Default transport type to use
    pub default_transport: TransportType,
    /// Whether to automatically upgrade to WebSocket for streaming methods
    pub auto_upgrade_streaming: bool,
    /// Transport configuration
    pub transport_config: TransportConfig,
}

impl Default for PeerManagerConfig {
    fn default() -> Self {
        Self {
            default_transport: TransportType::Http,
            auto_upgrade_streaming: true,
            transport_config: TransportConfig::default(),
        }
    }
}

/// Manages connections to peer agents
pub struct PeerManager {
    peers: RwLock<HashMap<String, PeerConnection>>,
    our_agent_id: String,
    config: PeerManagerConfig,
}

struct PeerConnection {
    url: String,
    http_transport: Option<Arc<HttpTransport>>,
    ws_transport: Option<Arc<WebSocketTransport>>,
    custom_transport: Option<A2aTransportRef>,
    transport_type: TransportType,
}

/// Helper enum for transport references to avoid holding locks across await
enum TransportRef {
    Http(Arc<HttpTransport>),
    WebSocket(Arc<WebSocketTransport>),
    Custom(A2aTransportRef),
}

impl PeerManager {
    /// Create a new peer manager with default configuration
    pub fn new(agent_id: String) -> Self {
        Self::with_config(agent_id, PeerManagerConfig::default())
    }

    /// Create a new peer manager with custom configuration
    pub fn with_config(agent_id: String, config: PeerManagerConfig) -> Self {
        Self {
            peers: RwLock::new(HashMap::new()),
            our_agent_id: agent_id,
            config,
        }
    }

    /// Determine if a method requires streaming/WebSocket transport
    fn is_streaming_method(method: &str) -> bool {
        // Methods that require WebSocket for streaming
        method.ends_with("_stream") || method.contains("/stream")
    }

    /// Determine the appropriate transport type for a method
    pub fn select_transport_for_method(&self, method: &str) -> TransportType {
        if self.config.auto_upgrade_streaming && Self::is_streaming_method(method) {
            TransportType::WebSocket
        } else {
            self.config.default_transport
        }
    }

    /// Connect to a list of peer URLs
    pub async fn connect_to_peers(
        &self,
        peer_urls: &[String],
    ) -> Result<(), Box<dyn std::error::Error>> {
        for url in peer_urls {
            if let Err(e) = self.connect_to_peer(url).await {
                warn!("Failed to connect to peer {}: {}", url, e);
            }
        }
        Ok(())
    }

    /// Connect to a single peer with specified transport type
    async fn connect_to_peer_with_transport(
        &self,
        url: &str,
        transport_type: TransportType,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Normalize URL based on transport type
        let normalized_url = match transport_type {
            TransportType::Http => {
                if url.starts_with("http://") || url.starts_with("https://") {
                    url.to_string()
                } else {
                    format!("http://{url}")
                }
            }
            TransportType::WebSocket => {
                if url.starts_with("ws://") || url.starts_with("wss://") {
                    url.to_string()
                } else if url.starts_with("http://") {
                    url.replace("http://", "ws://")
                } else if url.starts_with("https://") {
                    url.replace("https://", "wss://")
                } else {
                    format!("ws://{url}")
                }
            }
            TransportType::Custom => url.to_string(),
        };

        debug!(
            "Connecting to peer: {} with transport: {:?}",
            normalized_url, transport_type
        );

        let config = self.config.transport_config.clone();

        match transport_type {
            TransportType::Http => {
                let transport = HttpTransport::new(config)?;
                let endpoint = A2aEndpoint {
                    url: normalized_url.clone(),
                    agent_id: self.our_agent_id.clone(),
                    public_key: None,
                };

                transport.connect(&endpoint).await?;
                info!("Connected to peer via HTTP: {}", normalized_url);

                let mut peers = self.peers.write().unwrap();
                peers.insert(
                    url.to_string(),
                    PeerConnection {
                        url: normalized_url,
                        http_transport: Some(Arc::new(transport)),
                        ws_transport: None,
                        custom_transport: None,
                        transport_type: TransportType::Http,
                    },
                );
            }
            TransportType::WebSocket => {
                let transport = WebSocketTransport::new(config);
                let endpoint = A2aEndpoint {
                    url: normalized_url.clone(),
                    agent_id: self.our_agent_id.clone(),
                    public_key: None,
                };

                transport.connect(&endpoint).await?;
                info!("Connected to peer via WebSocket: {}", normalized_url);

                let mut peers = self.peers.write().unwrap();
                peers.insert(
                    url.to_string(),
                    PeerConnection {
                        url: normalized_url,
                        http_transport: None,
                        ws_transport: Some(Arc::new(transport)),
                        custom_transport: None,
                        transport_type: TransportType::WebSocket,
                    },
                );
            }
            TransportType::Custom => {
                // Custom transports are registered via register_transport()
                return Err("Custom transports must be registered via register_transport()".into());
            }
        }

        Ok(())
    }

    /// Connect to a single peer using default transport
    async fn connect_to_peer(&self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.connect_to_peer_with_transport(url, self.config.default_transport)
            .await
    }

    /// Ensure a peer has the required transport type, upgrading if necessary
    async fn ensure_transport(
        &self,
        peer_url: &str,
        required_transport: TransportType,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let needs_upgrade = {
            let peers = self.peers.read().unwrap();
            if let Some(peer) = peers.get(peer_url) {
                // Custom transports are provided externally and should not be
                // replaced or upgraded by the peer manager.
                if peer.transport_type == TransportType::Custom {
                    return Ok(());
                }
                peer.transport_type != required_transport
            } else {
                // Peer not connected at all
                return self
                    .connect_to_peer_with_transport(peer_url, required_transport)
                    .await;
            }
        };

        if needs_upgrade {
            debug!(
                "Upgrading peer {} to {:?} transport",
                peer_url, required_transport
            );
            self.connect_to_peer_with_transport(peer_url, required_transport)
                .await?;
        }

        Ok(())
    }

    /// Broadcast a message to all connected peers
    pub async fn broadcast(&self, method: &str, params: Value) -> Vec<Result<A2aResponse, String>> {
        let mut results = Vec::new();

        // Determine required transport for this method
        let required_transport = self.select_transport_for_method(method);

        let peer_urls: Vec<String> = {
            let peers = self.peers.read().unwrap();
            peers.keys().cloned().collect()
        };

        for peer_url in peer_urls {
            // Ensure peer has the required transport
            if let Err(e) = self.ensure_transport(&peer_url, required_transport).await {
                results.push(Err(format!(
                    "Failed to ensure transport for {peer_url}: {e}"
                )));
                continue;
            }

            let request = A2aRequest::new(method, params.clone());

            // Get Arc reference outside the lock to avoid holding lock across await
            let transport_ref = {
                let peers = self.peers.read().unwrap();
                if let Some(peer) = peers.get(&peer_url) {
                    if let Some(http) = &peer.http_transport {
                        Some(TransportRef::Http(Arc::clone(http)))
                    } else if let Some(ws) = &peer.ws_transport {
                        Some(TransportRef::WebSocket(Arc::clone(ws)))
                    } else {
                        peer.custom_transport
                            .as_ref()
                            .map(|c| TransportRef::Custom(Arc::clone(c)))
                    }
                } else {
                    None
                }
            };

            let result = match transport_ref {
                Some(TransportRef::Http(http)) => http.send_request(request).await,
                Some(TransportRef::WebSocket(ws)) => ws.send_request(request).await,
                Some(TransportRef::Custom(custom)) => custom.send_request(request).await,
                None => Err(anyhow::anyhow!("No transport available")),
            };

            match result {
                Ok(response) => results.push(Ok(response)),
                Err(e) => results.push(Err(format!("Failed to send to {peer_url}: {e}"))),
            }
        }

        results
    }

    /// Send a request to a specific peer
    pub async fn send_to(
        &self,
        peer_url: &str,
        method: &str,
        params: Value,
    ) -> Result<A2aResponse, Box<dyn std::error::Error>> {
        // Determine required transport for this method
        let required_transport = self.select_transport_for_method(method);

        // Ensure peer has the required transport
        self.ensure_transport(peer_url, required_transport).await?;

        let request = A2aRequest::new(method, params);

        // Get Arc reference outside the lock to avoid holding lock across await
        let transport_ref = {
            let peers = self.peers.read().unwrap();
            let peer = peers
                .get(peer_url)
                .ok_or_else(|| format!("Peer not found: {peer_url}"))?;

            if let Some(http) = &peer.http_transport {
                Ok(TransportRef::Http(Arc::clone(http)))
            } else if let Some(ws) = &peer.ws_transport {
                Ok(TransportRef::WebSocket(Arc::clone(ws)))
            } else if let Some(custom) = &peer.custom_transport {
                Ok(TransportRef::Custom(Arc::clone(custom)))
            } else {
                Err("No transport available")
            }
        }?;

        let response = match transport_ref {
            TransportRef::Http(http) => http.send_request(request).await?,
            TransportRef::WebSocket(ws) => ws.send_request(request).await?,
            TransportRef::Custom(custom) => custom.send_request(request).await?,
        };

        Ok(response)
    }

    /// Get list of connected peer URLs
    pub fn connected_peers(&self) -> Vec<String> {
        let peers = self.peers.read().unwrap();
        peers.values().map(|p| p.url.clone()).collect()
    }

    /// Get list of peers with their transport types
    pub fn connected_peers_with_transport(&self) -> Vec<(String, TransportType)> {
        let peers = self.peers.read().unwrap();
        peers
            .values()
            .map(|p| (p.url.clone(), p.transport_type))
            .collect()
    }

    /// Check if we have any connected peers
    pub fn has_peers(&self) -> bool {
        let peers = self.peers.read().unwrap();
        !peers.is_empty()
    }

    /// Get the number of connected peers
    pub fn peer_count(&self) -> usize {
        let peers = self.peers.read().unwrap();
        peers.len()
    }

    /// Get the transport type for a specific peer
    pub fn get_peer_transport_type(&self, peer_url: &str) -> Option<TransportType> {
        let peers = self.peers.read().unwrap();
        peers.get(peer_url).map(|p| p.transport_type)
    }

    /// Register an externally-created transport for a peer.
    ///
    /// This allows protocol adapters (e.g., OpenClaw WS bridge) to provide
    /// their own `A2aTransport` implementation without the `PeerManager`
    /// needing to know about the specific transport type.
    pub fn register_transport(&self, peer_url: &str, transport: A2aTransportRef) {
        info!("Registering custom transport for peer: {}", peer_url);
        let mut peers = self.peers.write().unwrap();
        peers.insert(
            peer_url.to_string(),
            PeerConnection {
                url: peer_url.to_string(),
                http_transport: None,
                ws_transport: None,
                custom_transport: Some(transport),
                transport_type: TransportType::Custom,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use uuid::Uuid;

    #[spec("PROTO-011")]
    #[spec("PROTO-018")]
    #[test]
    fn test_peer_manager_creation() {
        let manager = PeerManager::new("test-agent".to_string());
        assert!(!manager.has_peers());
        assert_eq!(manager.peer_count(), 0);
    }

    #[spec("PROTO-011")]
    #[spec("PROTO-018")]
    #[test]
    fn test_peer_manager_with_config() {
        let config = PeerManagerConfig {
            default_transport: TransportType::WebSocket,
            auto_upgrade_streaming: true,
            transport_config: TransportConfig::default(),
        };
        let manager = PeerManager::with_config("test-agent".to_string(), config);
        assert!(!manager.has_peers());
        assert_eq!(manager.peer_count(), 0);
    }

    #[spec("PROTO-014")]
    #[test]
    fn test_is_streaming_method() {
        assert!(PeerManager::is_streaming_method("chat_stream"));
        assert!(PeerManager::is_streaming_method("message/stream"));
        assert!(PeerManager::is_streaming_method("data_stream"));
        assert!(PeerManager::is_streaming_method("events/stream"));
        assert!(!PeerManager::is_streaming_method("agent_query"));
        assert!(!PeerManager::is_streaming_method("task_request"));
    }

    #[spec("PROTO-012")]
    #[spec("PROTO-014")]
    #[test]
    fn test_transport_selection() {
        let config = PeerManagerConfig {
            default_transport: TransportType::Http,
            auto_upgrade_streaming: true,
            transport_config: TransportConfig::default(),
        };
        let manager = PeerManager::with_config("test-agent".to_string(), config);

        // Streaming methods should select WebSocket
        assert_eq!(
            manager.select_transport_for_method("chat_stream"),
            TransportType::WebSocket
        );
        assert_eq!(
            manager.select_transport_for_method("message/stream"),
            TransportType::WebSocket
        );

        // Non-streaming methods should use default (HTTP)
        assert_eq!(
            manager.select_transport_for_method("agent_query"),
            TransportType::Http
        );
        assert_eq!(
            manager.select_transport_for_method("task_request"),
            TransportType::Http
        );
    }

    #[spec("PROTO-012")]
    #[spec("PROTO-014")]
    #[test]
    fn test_transport_selection_no_auto_upgrade() {
        let config = PeerManagerConfig {
            default_transport: TransportType::Http,
            auto_upgrade_streaming: false,
            transport_config: TransportConfig::default(),
        };
        let manager = PeerManager::with_config("test-agent".to_string(), config);

        // Even streaming methods should use default when auto_upgrade is false
        assert_eq!(
            manager.select_transport_for_method("chat_stream"),
            TransportType::Http
        );
        assert_eq!(
            manager.select_transport_for_method("agent_query"),
            TransportType::Http
        );
    }

    #[spec("PROTO-013")]
    #[test]
    fn test_websocket_default_transport() {
        let config = PeerManagerConfig {
            default_transport: TransportType::WebSocket,
            auto_upgrade_streaming: false,
            transport_config: TransportConfig::default(),
        };
        let manager = PeerManager::with_config("test-agent".to_string(), config);

        // All methods should use WebSocket when it's the default
        assert_eq!(
            manager.select_transport_for_method("chat_stream"),
            TransportType::WebSocket
        );
        assert_eq!(
            manager.select_transport_for_method("agent_query"),
            TransportType::WebSocket
        );
    }

    #[spec("PROTO-017")]
    #[spec("PROTO-018")]
    #[test]
    fn test_connected_peers_empty() {
        let manager = PeerManager::new("agent".to_string());
        let peers = manager.connected_peers();
        assert!(peers.is_empty());
    }

    /// Mock transport that records requests and returns a configurable response.
    struct MockTransport {
        responses: std::sync::Mutex<Vec<Result<A2aResponse, anyhow::Error>>>,
        requests: std::sync::Mutex<Vec<A2aRequest>>,
    }

    impl MockTransport {
        fn new(responses: Vec<Result<A2aResponse, anyhow::Error>>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
                requests: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn take_requests(&self) -> Vec<A2aRequest> {
            self.requests.lock().unwrap().drain(..).collect()
        }
    }

    #[async_trait::async_trait]
    impl A2aTransport for MockTransport {
        async fn connect(&self, _endpoint: &A2aEndpoint) -> anyhow::Result<()> {
            Ok(())
        }

        async fn send_request(&self, request: A2aRequest) -> anyhow::Result<A2aResponse> {
            self.requests.lock().unwrap().push(request);
            self.responses.lock().unwrap().remove(0)
        }

        async fn close(&self) -> anyhow::Result<()> {
            Ok(())
        }

        fn is_connected(&self) -> bool {
            true
        }
    }

    #[spec("PROTO-015")]
    #[tokio::test]
    async fn test_broadcast_to_registered_peers() {
        let manager = PeerManager::new("agent".to_string());

        let peer_a = Arc::new(MockTransport::new(vec![Ok(A2aResponse::Success {
            jsonrpc: "2.0".to_string(),
            id: Uuid::new_v4(),
            result: serde_json::json!({"peer": "a"}),
        })]));
        let peer_b = Arc::new(MockTransport::new(vec![Ok(A2aResponse::Success {
            jsonrpc: "2.0".to_string(),
            id: Uuid::new_v4(),
            result: serde_json::json!({"peer": "b"}),
        })]));

        manager.register_transport("http://peer-a.example", peer_a.clone());
        manager.register_transport("http://peer-b.example", peer_b.clone());

        let results = manager
            .broadcast("agent_query", serde_json::json!({"key": "value"}))
            .await;

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.is_ok()));

        let a_requests = peer_a.take_requests();
        let b_requests = peer_b.take_requests();
        assert_eq!(a_requests.len(), 1);
        assert_eq!(b_requests.len(), 1);
        assert_eq!(a_requests[0].method, "agent_query");
        assert_eq!(b_requests[0].method, "agent_query");
    }

    #[spec("PROTO-016")]
    #[tokio::test]
    async fn test_send_to_specific_peer() {
        let manager = PeerManager::new("agent".to_string());

        let peer = Arc::new(MockTransport::new(vec![Ok(A2aResponse::Success {
            jsonrpc: "2.0".to_string(),
            id: Uuid::new_v4(),
            result: serde_json::json!({"peer": "a"}),
        })]));

        manager.register_transport("http://peer-a.example", peer.clone());

        let response = manager
            .send_to(
                "http://peer-a.example",
                "agent_query",
                serde_json::json!({"key": "value"}),
            )
            .await;

        assert!(response.is_ok());
        let requests = peer.take_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "agent_query");
    }

    #[spec("PROTO-019")]
    #[tokio::test]
    async fn test_connect_to_peers_empty_list() {
        let manager = PeerManager::new("agent".to_string());
        let result = manager.connect_to_peers(&[]).await;
        assert!(result.is_ok());
        assert!(!manager.has_peers());
    }

    #[spec("PROTO-019")]
    #[tokio::test]
    async fn test_connect_to_peers_continues_on_failure() {
        let manager = PeerManager::new("agent".to_string());
        // An invalid URL should fail to connect but not abort the loop.
        let result = manager
            .connect_to_peers(&["not-a-valid-url".to_string()])
            .await;
        assert!(result.is_ok());
    }
}
