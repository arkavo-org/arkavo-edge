//! Peer Manager for A2A Agent Communication
//!
//! Manages connections to peer agents for agent-to-agent communication.

use arkavo_protocol::http::HttpTransport;
use arkavo_protocol::transport::{
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::RwLock;
use tracing::{debug, info, warn};

/// Manages connections to peer agents
pub struct PeerManager {
    peers: RwLock<HashMap<String, PeerConnection>>,
    our_agent_id: String,
}

struct PeerConnection {
    url: String,
    transport: HttpTransport,
}

impl PeerManager {
    /// Create a new peer manager
    pub fn new(agent_id: String) -> Self {
        Self {
            peers: RwLock::new(HashMap::new()),
            our_agent_id: agent_id,
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

    /// Connect to a single peer
    async fn connect_to_peer(&self, url: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Normalize URL
        let normalized_url = if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            format!("http://{}", url)
        };

        debug!("Connecting to peer: {}", normalized_url);

        let config = TransportConfig::default();
        let transport = HttpTransport::new(config)?;

        let endpoint = A2aEndpoint {
            url: normalized_url.clone(),
            agent_id: self.our_agent_id.clone(),
            public_key: None,
        };

        transport.connect(&endpoint).await?;

        info!("Connected to peer: {}", normalized_url);

        {
            let mut peers = self.peers.write().unwrap();
            peers.insert(
                normalized_url.clone(),
                PeerConnection {
                    url: normalized_url,
                    transport,
                },
            );
        }

        Ok(())
    }

    /// Broadcast a message to all connected peers
    #[allow(dead_code)]
    pub async fn broadcast(&self, method: &str, params: Value) -> Vec<Result<A2aResponse, String>> {
        let mut results = Vec::new();

        let peers = self.peers.read().unwrap();
        for (_, peer) in peers.iter() {
            let request = A2aRequest::new(method, params.clone());
            match peer.transport.send_request(request).await {
                Ok(response) => results.push(Ok(response)),
                Err(e) => results.push(Err(format!("Failed to send to {}: {}", peer.url, e))),
            }
        }

        results
    }

    /// Send a request to a specific peer
    #[allow(dead_code)]
    pub async fn send_to(
        &self,
        peer_url: &str,
        method: &str,
        params: Value,
    ) -> Result<A2aResponse, Box<dyn std::error::Error>> {
        let peers = self.peers.read().unwrap();
        let peer = peers
            .get(peer_url)
            .ok_or_else(|| format!("Peer not found: {}", peer_url))?;

        let request = A2aRequest::new(method, params);
        let response = peer.transport.send_request(request).await?;
        Ok(response)
    }

    /// Get list of connected peer URLs
    #[allow(dead_code)]
    pub fn connected_peers(&self) -> Vec<String> {
        let peers = self.peers.read().unwrap();
        peers.values().map(|p| p.url.clone()).collect()
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_manager_creation() {
        let manager = PeerManager::new("test-agent".to_string());
        assert!(!manager.has_peers());
        assert_eq!(manager.peer_count(), 0);
    }
}
