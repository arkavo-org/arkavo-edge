//! Server-side configuration transport
//!
//! Provides functionality for orchestrators to distribute configuration bundles
//! to agents over the A2A protocol.

use crate::{BundleRegistry, ConfigTransportEnvelope, Result, TransportError};
use arkavo_config_bundle::ConfigurationBundle;
use arkavo_config_encryption::{ConfigBundleEncryptor, EncryptedBundle, KeyPair, Policy};
use arkavo_protocol::types::{AgentConfigGetRequest, AgentConfigGetResponse};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Server for distributing configuration bundles over A2A protocol
pub struct ConfigTransportServer {
    encryptor: ConfigBundleEncryptor,
    registry: Arc<RwLock<BundleRegistry>>,
    orchestrator_key_pair: KeyPair,
    kas_url: String,
}

impl ConfigTransportServer {
    /// Create a new configuration transport server
    pub fn new(kas_url: String) -> Result<Self> {
        let orchestrator_key_pair =
            KeyPair::generate().map_err(|e| TransportError::Encryption(e.to_string()))?;

        Ok(Self {
            encryptor: ConfigBundleEncryptor::new(kas_url.clone()),
            registry: Arc::new(RwLock::new(BundleRegistry::new())),
            orchestrator_key_pair,
            kas_url,
        })
    }

    /// Distribute a configuration bundle to an agent
    ///
    /// This method:
    /// 1. Validates the bundle
    /// 2. Encrypts it with OpenTDF-style encryption
    /// 3. Signs the encrypted bundle
    /// 4. Wraps it in a transport envelope
    /// 5. Registers it for distribution
    pub async fn distribute_bundle(
        &self,
        bundle: ConfigurationBundle,
        policy: Policy,
    ) -> Result<String> {
        info!(
            bundle_id = %bundle.bundle_id,
            target = ?bundle.target,
            "Distributing configuration bundle"
        );

        // Validate bundle
        bundle
            .validate()
            .map_err(|e| TransportError::Validation(e.to_string()))?;

        // Encrypt bundle
        let encrypted_bundle = self
            .encryptor
            .encrypt_bundle(&bundle, policy)
            .map_err(|e| TransportError::Encryption(e.to_string()))?;

        debug!(
            bundle_id = %encrypted_bundle.bundle_id,
            "Bundle encrypted successfully"
        );

        // Sign encrypted bundle
        let bundle_json = serde_json::to_string(&encrypted_bundle)?;
        let signature = self
            .orchestrator_key_pair
            .private_key
            .sign(bundle_json.as_bytes())
            .map_err(|e| TransportError::Encryption(e.to_string()))?;

        debug!(
            bundle_id = %encrypted_bundle.bundle_id,
            signature_len = signature.len(),
            "Bundle signed successfully"
        );

        // Create transport envelope
        let envelope = ConfigTransportEnvelope::new(
            &encrypted_bundle,
            signature,
            self.kas_url.clone(),
        )?;

        let bundle_id = encrypted_bundle.bundle_id.to_string();

        // Register bundle
        let mut registry = self.registry.write().await;
        registry.register_bundle(bundle_id.clone(), encrypted_bundle, envelope);

        info!(
            bundle_id = %bundle_id,
            "Bundle registered for distribution"
        );

        Ok(bundle_id)
    }

    /// Handle agent configuration request
    ///
    /// This is called by the A2A RPC handler when an agent requests configuration
    pub async fn handle_config_request(
        &self,
        request: &AgentConfigGetRequest,
        agent_public_key: &[u8],
    ) -> Result<AgentConfigGetResponse> {
        info!(
            agent_id = %request.agent_id,
            "Handling configuration request"
        );

        // Find bundle for this agent
        let registry = self.registry.read().await;
        let bundles = registry.list_bundles();

        // In a real implementation, this would:
        // 1. Look up agent's assigned bundle based on agent_id or attributes
        // 2. Verify agent's signature on the request
        // 3. Check agent's public key against registered keys

        if bundles.is_empty() {
            warn!(
                agent_id = %request.agent_id,
                "No configuration bundles available"
            );
            return Err(TransportError::Transport(
                "No configuration available".to_string(),
            ));
        }

        // For now, return the first bundle (in production, this would be filtered)
        let bundle_id = &bundles[0];
        let envelope = registry
            .get_envelope(bundle_id)
            .ok_or_else(|| TransportError::Transport("Bundle not found".to_string()))?;

        debug!(
            agent_id = %request.agent_id,
            bundle_id = %bundle_id,
            "Found configuration bundle for agent"
        );

        // Verify agent's public key (in production)
        // This would check if agent_public_key matches registered key

        // Convert envelope to config response
        let content = serde_json::to_string_pretty(envelope)?;

        Ok(AgentConfigGetResponse {
            content,
            version: envelope.version.clone(),
            backups: None,
            writable: false,
        })
    }

    /// Get orchestrator's public key for agent verification
    pub fn get_public_key(&self) -> Vec<u8> {
        self.orchestrator_key_pair.public_key.as_bytes().to_vec()
    }

    /// List all registered bundles
    pub async fn list_bundles(&self) -> Vec<String> {
        let registry = self.registry.read().await;
        registry.list_bundles()
    }

    /// Get bundle details
    pub async fn get_bundle(&self, bundle_id: &str) -> Option<EncryptedBundle> {
        let registry = self.registry.read().await;
        registry.get_bundle(bundle_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_config_bundle::{AgentRole, BundleTarget};

    #[tokio::test]
    async fn test_server_creation() {
        let server = ConfigTransportServer::new("https://kas.example.com".to_string()).unwrap();
        assert!(!server.get_public_key().is_empty());
    }

    #[tokio::test]
    async fn test_distribute_bundle() {
        let server = ConfigTransportServer::new("https://kas.example.com".to_string()).unwrap();

        let mut bundle = ConfigurationBundle::new(
            BundleTarget::Role("test-agent".to_string()),
            AgentRole {
                name: "test-agent".to_string(),
                purpose: "Testing".to_string(),
                capabilities: vec!["test".to_string()],
            },
            "admin".to_string(),
            "test".to_string(),
        );

        bundle.add_required_attribute("agent.role=test-agent".to_string());

        let mut policy = Policy::new();
        policy.add_attribute("agent.role".to_string(), "Agent Role".to_string());
        policy.add_dissemination("agent.role=test-agent".to_string());

        let bundle_id = server.distribute_bundle(bundle, policy).await.unwrap();
        assert!(!bundle_id.is_empty());

        let bundles = server.list_bundles().await;
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0], bundle_id);
    }
}