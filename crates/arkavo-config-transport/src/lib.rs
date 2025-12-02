//! Configuration transport layer using A2A protocol
//!
//! This crate provides secure configuration bundle transport over the existing
//! A2A protocol infrastructure, leveraging the agent.config.* RPC methods.

use arkavo_config_bundle::ConfigurationBundle;
use arkavo_config_encryption::{
    AgentIdentity, ConfigBundleDecryptor, ConfigBundleEncryptor, EncryptedBundle, Policy,
};
use arkavo_protocol::types::{
    AgentConfigGetRequest, AgentConfigGetResponse, AgentConfigUpdateRequest,
    AgentConfigUpdateResponse,
};
use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use tracing::{debug, error, info};

pub mod client;
pub mod server;

pub use client::ConfigTransportClient;
pub use server::ConfigTransportServer;

#[derive(Error, Debug)]
pub enum TransportError {
    #[error("Encryption error: {0}")]
    Encryption(String),

    #[error("Decryption error: {0}")]
    Decryption(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Authorization error: {0}")]
    Authorization(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Base64 decode error: {0}")]
    Base64Decode(#[from] base64::DecodeError),
}

pub type Result<T> = std::result::Result<T, TransportError>;

/// Wrapper for transporting encrypted bundles over A2A protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigTransportEnvelope {
    /// Version of the transport protocol
    pub version: String,

    /// Type of payload (always "encrypted_bundle")
    pub payload_type: String,

    /// Base64-encoded encrypted bundle
    pub encrypted_payload: String,

    /// Signature of the payload (signed by orchestrator)
    pub signature: String,

    /// Timestamp of creation
    pub timestamp: chrono::DateTime<Utc>,

    /// Metadata for routing and validation
    pub metadata: TransportMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportMetadata {
    /// Bundle ID for tracking
    pub bundle_id: String,

    /// Target agent ID or pattern
    pub target: String,

    /// KAS URL for key retrieval
    pub kas_url: String,

    /// Required attributes for access
    pub required_attributes: Vec<String>,
}

impl ConfigTransportEnvelope {
    /// Create a new transport envelope from an encrypted bundle
    pub fn new(
        encrypted_bundle: &EncryptedBundle,
        signature: Vec<u8>,
        kas_url: String,
    ) -> Result<Self> {
        let bundle_json = serde_json::to_vec(encrypted_bundle)?;
        let encrypted_payload = base64::engine::general_purpose::STANDARD.encode(&bundle_json);
        let signature_b64 = base64::engine::general_purpose::STANDARD.encode(&signature);

        Ok(Self {
            version: "1.0.0".to_string(),
            payload_type: "encrypted_bundle".to_string(),
            encrypted_payload,
            signature: signature_b64,
            timestamp: Utc::now(),
            metadata: TransportMetadata {
                bundle_id: encrypted_bundle.bundle_id.to_string(),
                target: encrypted_bundle.target.clone(),
                kas_url,
                required_attributes: encrypted_bundle
                    .policy_manifest
                    .dissemination
                    .clone(),
            },
        })
    }

    /// Extract the encrypted bundle from the envelope
    pub fn extract_bundle(&self) -> Result<EncryptedBundle> {
        let bundle_bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.encrypted_payload)?;
        let bundle: EncryptedBundle = serde_json::from_slice(&bundle_bytes)?;
        Ok(bundle)
    }

    /// Verify the signature of the envelope
    pub fn verify_signature(&self, public_key: &[u8]) -> Result<bool> {
        use arkavo_config_encryption::PublicKey;

        let signature_bytes = base64::engine::general_purpose::STANDARD
            .decode(&self.signature)?;

        let public_key = PublicKey::from_bytes(public_key.to_vec());

        public_key
            .verify(self.encrypted_payload.as_bytes(), &signature_bytes)
            .map_err(|e| TransportError::Authorization(e.to_string()))
    }

    /// Convert to A2A config update request
    pub fn to_config_update_request(&self, agent_id: String) -> Result<AgentConfigUpdateRequest> {
        let content = serde_json::to_string_pretty(self)?;

        Ok(AgentConfigUpdateRequest {
            agent_id,
            content,
            expected_version: None,
            create_backup: true,
        })
    }

    /// Parse from A2A config get response
    pub fn from_config_response(response: &AgentConfigGetResponse) -> Result<Self> {
        let envelope: ConfigTransportEnvelope = serde_json::from_str(&response.content)?;
        Ok(envelope)
    }
}

/// Configuration bundle registry for tracking distributed bundles
#[derive(Debug, Clone)]
pub struct BundleRegistry {
    bundles: HashMap<String, EncryptedBundle>,
    envelopes: HashMap<String, ConfigTransportEnvelope>,
}

impl BundleRegistry {
    pub fn new() -> Self {
        Self {
            bundles: HashMap::new(),
            envelopes: HashMap::new(),
        }
    }

    pub fn register_bundle(
        &mut self,
        bundle_id: String,
        bundle: EncryptedBundle,
        envelope: ConfigTransportEnvelope,
    ) {
        self.bundles.insert(bundle_id.clone(), bundle);
        self.envelopes.insert(bundle_id, envelope);
    }

    pub fn get_bundle(&self, bundle_id: &str) -> Option<&EncryptedBundle> {
        self.bundles.get(bundle_id)
    }

    pub fn get_envelope(&self, bundle_id: &str) -> Option<&ConfigTransportEnvelope> {
        self.envelopes.get(bundle_id)
    }

    pub fn list_bundles(&self) -> Vec<String> {
        self.bundles.keys().cloned().collect()
    }
}

impl Default for BundleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_config_bundle::{AgentRole, BundleTarget};
    use arkavo_config_encryption::KeyPair;

    #[test]
    fn test_transport_envelope_creation() {
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

        let encryptor = ConfigBundleEncryptor::new("https://kas.example.com".to_string());
        let mut policy = Policy::new();
        policy.add_attribute(
            "agent/identity".to_string(),
            "autonomous_service".to_string(),
            "Agent Identity".to_string(),
        );
        policy.add_dissemination("agent.role=test-agent".to_string());

        let encrypted = encryptor.encrypt_bundle(&bundle, policy).unwrap();

        let key_pair = KeyPair::generate().unwrap();
        let signature = key_pair
            .private_key
            .sign(serde_json::to_string(&encrypted).unwrap().as_bytes())
            .unwrap();

        let envelope = ConfigTransportEnvelope::new(
            &encrypted,
            signature,
            "https://kas.example.com".to_string(),
        )
        .unwrap();

        assert_eq!(envelope.version, "1.0.0");
        assert_eq!(envelope.payload_type, "encrypted_bundle");
        assert!(!envelope.encrypted_payload.is_empty());
        assert!(!envelope.signature.is_empty());
    }

    #[test]
    fn test_extract_bundle() {
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

        let encryptor = ConfigBundleEncryptor::new("https://kas.example.com".to_string());
        let mut policy = Policy::new();
        policy.add_attribute(
            "agent/identity".to_string(),
            "autonomous_service".to_string(),
            "Agent Identity".to_string(),
        );
        policy.add_dissemination("agent.role=test-agent".to_string());

        let encrypted = encryptor.encrypt_bundle(&bundle, policy).unwrap();

        let key_pair = KeyPair::generate().unwrap();
        let signature = key_pair
            .private_key
            .sign(serde_json::to_string(&encrypted).unwrap().as_bytes())
            .unwrap();

        let envelope = ConfigTransportEnvelope::new(
            &encrypted,
            signature,
            "https://kas.example.com".to_string(),
        )
        .unwrap();

        let extracted = envelope.extract_bundle().unwrap();
        assert_eq!(extracted.bundle_id, encrypted.bundle_id);
    }

    #[test]
    fn test_bundle_registry() {
        let mut registry = BundleRegistry::new();

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

        let encryptor = ConfigBundleEncryptor::new("https://kas.example.com".to_string());
        let mut policy = Policy::new();
        policy.add_attribute(
            "agent/identity".to_string(),
            "autonomous_service".to_string(),
            "Agent Identity".to_string(),
        );
        policy.add_dissemination("agent.role=test-agent".to_string());

        let encrypted = encryptor.encrypt_bundle(&bundle, policy).unwrap();
        let bundle_id = encrypted.bundle_id.to_string();

        let key_pair = KeyPair::generate().unwrap();
        let signature = key_pair
            .private_key
            .sign(serde_json::to_string(&encrypted).unwrap().as_bytes())
            .unwrap();

        let envelope = ConfigTransportEnvelope::new(
            &encrypted,
            signature,
            "https://kas.example.com".to_string(),
        )
        .unwrap();

        registry.register_bundle(bundle_id.clone(), encrypted, envelope);

        assert!(registry.get_bundle(&bundle_id).is_some());
        assert!(registry.get_envelope(&bundle_id).is_some());
        assert_eq!(registry.list_bundles().len(), 1);
    }
}