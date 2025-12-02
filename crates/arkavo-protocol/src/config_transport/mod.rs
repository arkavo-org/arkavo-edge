//! Configuration transport integration for A2A protocol
//!
//! This module provides integration between arkavo-config-transport and the A2A RPC server,
//! enabling secure configuration bundle distribution over the existing A2A protocol.

pub mod integration;

use crate::Result;
use crate::types::{AgentConfigGetRequest, AgentConfigGetResponse};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

pub use integration::ConfigTransportIntegration;

/// Agent public key registry for signature verification
#[derive(Debug, Clone)]
pub struct AgentKeyRegistry {
    keys: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl AgentKeyRegistry {
    pub fn new() -> Self {
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register an agent's public key
    pub async fn register_public_key(&self, agent_id: String, public_key: Vec<u8>) {
        let mut keys = self.keys.write().await;
        keys.insert(agent_id, public_key);
    }

    /// Get an agent's public key
    pub async fn get_public_key(&self, agent_id: &str) -> Option<Vec<u8>> {
        let keys = self.keys.read().await;
        keys.get(agent_id).cloned()
    }

    /// Remove an agent's public key
    pub async fn remove_public_key(&self, agent_id: &str) {
        let mut keys = self.keys.write().await;
        keys.remove(agent_id);
    }

    /// List all registered agent IDs
    pub async fn list_agents(&self) -> Vec<String> {
        let keys = self.keys.read().await;
        keys.keys().cloned().collect()
    }
}

impl Default for AgentKeyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Configuration transport handler for A2A RPC
///
/// This handler can be integrated into A2aRpcImpl to provide secure
/// configuration distribution capabilities.
pub struct ConfigTransportHandler {
    key_registry: AgentKeyRegistry,
    kas_url: String,
}

impl ConfigTransportHandler {
    pub fn new(kas_url: String) -> Self {
        Self {
            key_registry: AgentKeyRegistry::new(),
            kas_url,
        }
    }

    /// Register an agent's public key for signature verification
    pub async fn register_agent_key(&self, agent_id: String, public_key: Vec<u8>) {
        info!(
            agent_id = %agent_id,
            key_len = public_key.len(),
            "Registering agent public key"
        );
        self.key_registry
            .register_public_key(agent_id, public_key)
            .await;
    }

    /// Handle agent configuration request with secure bundle transport
    ///
    /// This method can be called from A2aRpcImpl::agent_config_get to provide
    /// secure configuration bundle distribution.
    pub async fn handle_config_request(
        &self,
        request: &AgentConfigGetRequest,
    ) -> Result<Option<AgentConfigGetResponse>> {
        debug!(
            agent_id = %request.agent_id,
            "Handling secure configuration request"
        );

        // Get agent's public key
        let public_key = match self.key_registry.get_public_key(&request.agent_id).await {
            Some(key) => key,
            None => {
                warn!(
                    agent_id = %request.agent_id,
                    "Agent public key not registered - falling back to standard config"
                );
                return Ok(None); // Fall back to standard AGENTS.md config
            }
        };

        debug!(
            agent_id = %request.agent_id,
            key_len = public_key.len(),
            "Found agent public key"
        );

        // In a full implementation, this would:
        // 1. Look up the agent's assigned configuration bundle
        // 2. Use ConfigTransportServer to create the response
        // 3. Return the encrypted bundle wrapped in ConfigTransportEnvelope
        //
        // For now, we return None to indicate fallback to standard config
        // This allows gradual migration to secure bundles

        info!(
            agent_id = %request.agent_id,
            "Secure config transport available but no bundle assigned - using standard config"
        );

        Ok(None)
    }

    /// Get the KAS URL for this handler
    pub fn kas_url(&self) -> &str {
        &self.kas_url
    }

    /// Get the key registry
    pub fn key_registry(&self) -> &AgentKeyRegistry {
        &self.key_registry
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_key_registry() {
        let registry = AgentKeyRegistry::new();

        let agent_id = "test-agent-001".to_string();
        let public_key = vec![1, 2, 3, 4, 5];

        registry
            .register_public_key(agent_id.clone(), public_key.clone())
            .await;

        let retrieved = registry.get_public_key(&agent_id).await;
        assert_eq!(retrieved, Some(public_key));

        let agents = registry.list_agents().await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0], agent_id);

        registry.remove_public_key(&agent_id).await;
        let retrieved = registry.get_public_key(&agent_id).await;
        assert_eq!(retrieved, None);
    }

    #[tokio::test]
    async fn test_handler_creation() {
        let handler = ConfigTransportHandler::new("https://kas.example.com".to_string());
        assert_eq!(handler.kas_url(), "https://kas.example.com");
    }

    #[tokio::test]
    async fn test_handle_config_request_no_key() {
        let handler = ConfigTransportHandler::new("https://kas.example.com".to_string());

        let request = AgentConfigGetRequest {
            agent_id: "test-agent-001".to_string(),
            include_backups: false,
        };

        let result = handler.handle_config_request(&request).await.unwrap();
        assert!(result.is_none()); // Should fall back to standard config
    }

    #[tokio::test]
    async fn test_handle_config_request_with_key() {
        let handler = ConfigTransportHandler::new("https://kas.example.com".to_string());

        let agent_id = "test-agent-001".to_string();
        let public_key = vec![1, 2, 3, 4, 5];

        handler
            .register_agent_key(agent_id.clone(), public_key)
            .await;

        let request = AgentConfigGetRequest {
            agent_id,
            include_backups: false,
        };

        let result = handler.handle_config_request(&request).await.unwrap();
        // Currently returns None as we don't have bundle assignment logic yet
        assert!(result.is_none());
    }
}
