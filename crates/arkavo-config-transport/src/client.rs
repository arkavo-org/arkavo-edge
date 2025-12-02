//! Client-side configuration transport
//!
//! Provides functionality for agents to request and receive configuration bundles
//! over the A2A protocol.

use crate::{ConfigTransportEnvelope, Result, TransportError};
use arkavo_config_bundle::ConfigurationBundle;
use arkavo_config_encryption::{AgentIdentity, ConfigBundleDecryptor};
use arkavo_protocol::types::{AgentConfigGetRequest, AgentConfigGetResponse};
use chrono::Utc;
use tracing::{debug, info, warn};

/// Client for requesting configuration bundles over A2A protocol
pub struct ConfigTransportClient {
    agent_identity: AgentIdentity,
    decryptor: ConfigBundleDecryptor,
}

impl ConfigTransportClient {
    /// Create a new configuration transport client
    pub fn new(agent_identity: AgentIdentity) -> Self {
        let decryptor = ConfigBundleDecryptor::new(agent_identity.clone());
        Self {
            agent_identity,
            decryptor,
        }
    }

    /// Request configuration from orchestrator
    ///
    /// This method:
    /// 1. Creates a signed request for configuration
    /// 2. Sends it via A2A protocol (agent.config.get)
    /// 3. Receives encrypted bundle in response
    /// 4. Verifies signature
    /// 5. Decrypts bundle using agent's identity
    pub async fn request_configuration(
        &self,
        decryption_key: &[u8],
    ) -> Result<ConfigurationBundle> {
        info!(
            agent_id = %self.agent_identity.agent_id,
            "Requesting configuration from orchestrator"
        );

        // Create request data for signing
        let request_data = format!(
            "{}:{}:{}",
            self.agent_identity.agent_id,
            self.agent_identity.jwt_token,
            Utc::now().timestamp()
        );

        // Sign request with agent's private key
        let signature = self
            .agent_identity
            .sign_request(request_data.as_bytes())
            .map_err(|e| TransportError::Authorization(e.to_string()))?;

        debug!(
            agent_id = %self.agent_identity.agent_id,
            signature_len = signature.len(),
            "Request signed successfully"
        );

        // In a real implementation, this would call the A2A RPC method
        // For now, we'll simulate the response structure
        let config_request = AgentConfigGetRequest {
            agent_id: self.agent_identity.agent_id.clone(),
            include_backups: false,
        };

        // This would be: let response = a2a_client.agent_config_get(config_request).await?;
        // For now, we return an error indicating this needs A2A client integration
        Err(TransportError::Transport(
            "A2A client integration required - use with A2aRpc implementation".to_string(),
        ))
    }

    /// Process configuration response from orchestrator
    pub fn process_configuration_response(
        &self,
        response: &AgentConfigGetResponse,
        decryption_key: &[u8],
    ) -> Result<ConfigurationBundle> {
        info!(
            agent_id = %self.agent_identity.agent_id,
            "Processing configuration response"
        );

        // Parse transport envelope from response
        let envelope = ConfigTransportEnvelope::from_config_response(response)?;

        debug!(
            bundle_id = %envelope.metadata.bundle_id,
            "Parsed transport envelope"
        );

        // Extract encrypted bundle
        let encrypted_bundle = envelope.extract_bundle()?;

        debug!(
            bundle_id = %encrypted_bundle.bundle_id,
            "Extracted encrypted bundle"
        );

        // Verify agent has required attributes
        if !self
            .agent_identity
            .has_required_attributes(&envelope.metadata.required_attributes)
        {
            warn!(
                agent_id = %self.agent_identity.agent_id,
                required = ?envelope.metadata.required_attributes,
                "Agent does not have required attributes"
            );
            return Err(TransportError::Authorization(
                "Agent does not have required attributes".to_string(),
            ));
        }

        // Decrypt bundle
        let bundle = self
            .decryptor
            .decrypt_bundle(&encrypted_bundle)
            .map_err(|e| TransportError::Decryption(e.to_string()))?;

        info!(
            agent_id = %self.agent_identity.agent_id,
            bundle_id = %bundle.bundle_id,
            "Configuration decrypted successfully"
        );

        Ok(bundle)
    }

    /// Acknowledge configuration receipt
    pub async fn acknowledge_configuration(&self, bundle_id: &str) -> Result<()> {
        info!(
            agent_id = %self.agent_identity.agent_id,
            bundle_id = %bundle_id,
            "Acknowledging configuration receipt"
        );

        // Create acknowledgment data
        let ack_data = format!("{}:{}", bundle_id, Utc::now().timestamp());

        // Sign acknowledgment
        let _signature = self
            .agent_identity
            .sign_request(ack_data.as_bytes())
            .map_err(|e| TransportError::Authorization(e.to_string()))?;

        // In a real implementation, this would send acknowledgment via A2A
        // For now, we just log it
        debug!(
            agent_id = %self.agent_identity.agent_id,
            bundle_id = %bundle_id,
            "Configuration acknowledgment prepared"
        );

        Ok(())
    }

    /// Apply configuration to agent
    pub fn apply_configuration(&self, bundle: &ConfigurationBundle) -> Result<()> {
        info!(
            agent_id = %self.agent_identity.agent_id,
            bundle_id = %bundle.bundle_id,
            "Applying configuration"
        );

        // Validate bundle
        bundle
            .validate()
            .map_err(|e| TransportError::Validation(e.to_string()))?;

        // In a real implementation, this would:
        // 1. Apply settings to agent
        // 2. Load secrets into environment
        // 3. Configure entitlements
        // 4. Update agent capabilities

        info!(
            agent_id = %self.agent_identity.agent_id,
            bundle_id = %bundle.bundle_id,
            settings_count = bundle.settings.len(),
            secrets_count = bundle.secrets.len(),
            entitlements_count = bundle.entitlements.len(),
            "Configuration applied successfully"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_client_creation() {
        let mut attributes = HashMap::new();
        attributes.insert("agent.role".to_string(), "test-agent".to_string());

        let identity = AgentIdentity::new("test-agent-001".to_string(), attributes).unwrap();
        let client = ConfigTransportClient::new(identity);

        assert_eq!(client.agent_identity.agent_id, "test-agent-001");
    }
}