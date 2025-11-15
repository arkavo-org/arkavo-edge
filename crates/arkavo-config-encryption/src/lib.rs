use arkavo_config_bundle::ConfigurationBundle;
use chrono::{DateTime, Utc};
use opentdf::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

pub mod error;
pub mod kas;
pub mod keys;
pub mod policy;

pub use error::{Error, Result};
pub use kas::{DEFAULT_IDENTITY_URL, DEFAULT_KAS_URL, KasConfig};
pub use keys::{KeyPair, PrivateKey, PublicKey};
pub use policy::{Policy, PolicyAttribute};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedBundle {
    pub bundle_id: Uuid,
    pub target: String,
    pub encrypted_data: Vec<u8>,
    pub policy_manifest: PolicyManifest,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyManifest {
    pub policy_id: Uuid,
    pub data_attributes: Vec<PolicyAttribute>,
    pub dissemination: Vec<String>,
    pub kas_url: String,
}

pub struct ConfigBundleEncryptor {
    kas_url: String,
}

impl ConfigBundleEncryptor {
    pub fn new(kas_url: String) -> Self {
        Self { kas_url }
    }

    pub fn encrypt_bundle(
        &self,
        bundle: &ConfigurationBundle,
        policy: Policy,
    ) -> Result<EncryptedBundle> {
        bundle
            .validate()
            .map_err(|e| Error::Validation(e.to_string()))?;

        let plaintext =
            serde_json::to_vec(bundle).map_err(|e| Error::Serialization(e.to_string()))?;

        let mut tdf_policy = PolicyBuilder::new().id_auto();

        for attr in &policy.attributes {
            let fqn = format!("https://arkavo.com/attr/{}/value/active", attr.attribute);
            tdf_policy = tdf_policy
                .attribute_fqn(&fqn)
                .map_err(|e| Error::Policy(format!("Invalid attribute FQN: {}", e)))?;
        }

        for dissem in &policy.dissemination {
            tdf_policy = tdf_policy.dissemination([dissem.as_str()]);
        }

        let built_policy = tdf_policy
            .build()
            .map_err(|e| Error::Policy(format!("Failed to build policy: {}", e)))?;

        let policy_id_str = built_policy.uuid.clone();

        let encrypted_data = Tdf::encrypt(plaintext)
            .kas_url(&self.kas_url)
            .policy(built_policy)
            .to_bytes()
            .map_err(|e| Error::Encryption(format!("TDF encryption failed: {}", e)))?;

        let policy_id = Uuid::parse_str(&policy_id_str)
            .map_err(|e| Error::Policy(format!("Invalid policy UUID: {}", e)))?;

        let policy_manifest = PolicyManifest {
            policy_id,
            data_attributes: policy.attributes,
            dissemination: policy.dissemination,
            kas_url: self.kas_url.clone(),
        };

        Ok(EncryptedBundle {
            bundle_id: bundle.bundle_id,
            target: format!("{:?}", bundle.target),
            encrypted_data,
            policy_manifest,
            created_at: Utc::now(),
        })
    }
}

pub struct ConfigBundleDecryptor {
    agent_identity: AgentIdentity,
}

impl ConfigBundleDecryptor {
    pub fn new(agent_identity: AgentIdentity) -> Self {
        Self { agent_identity }
    }

    pub fn decrypt_bundle(&self, encrypted: &EncryptedBundle) -> Result<ConfigurationBundle> {
        if !self.verify_attributes(&encrypted.policy_manifest) {
            return Err(Error::Authorization(
                "Agent does not have required attributes".to_string(),
            ));
        }

        Err(Error::Decryption(
            "TDF decryption requires async KAS integration - use decrypt_bundle_async instead"
                .to_string(),
        ))
    }

    #[cfg(feature = "kas")]
    pub async fn decrypt_bundle_async(
        &self,
        encrypted: &EncryptedBundle,
        kas_client: opentdf::KasClient,
    ) -> Result<ConfigurationBundle> {
        if !self.verify_attributes(&encrypted.policy_manifest) {
            return Err(Error::Authorization(
                "Agent does not have required attributes".to_string(),
            ));
        }

        let tdf_bytes = encrypted.encrypted_data.clone();

        let plaintext = Tdf::decrypt(tdf_bytes)
            .kas_client(kas_client)
            .to_bytes()
            .await
            .map_err(|e| Error::Decryption(format!("TDF decryption failed: {}", e)))?;

        let bundle: ConfigurationBundle = serde_json::from_slice(&plaintext)
            .map_err(|e| Error::Deserialization(e.to_string()))?;

        Ok(bundle)
    }

    fn verify_attributes(&self, policy: &PolicyManifest) -> bool {
        policy.dissemination.iter().all(|required| {
            if let Some((key, value)) = required.split_once('=') {
                self.agent_identity
                    .attributes
                    .get(key)
                    .map(|v| v == value)
                    .unwrap_or(false)
            } else {
                false
            }
        })
    }
}

#[derive(Debug, Clone)]
pub struct AgentIdentity {
    pub agent_id: String,
    pub attributes: HashMap<String, String>,
    pub jwt_token: String,
    pub private_key: PrivateKey,
    pub public_key: PublicKey,
}

impl AgentIdentity {
    pub fn new(agent_id: String, attributes: HashMap<String, String>) -> Result<Self> {
        let key_pair = KeyPair::generate()?;
        Ok(Self {
            agent_id,
            attributes,
            jwt_token: String::new(),
            private_key: key_pair.private_key,
            public_key: key_pair.public_key,
        })
    }

    pub fn sign_request(&self, request_data: &[u8]) -> Result<Vec<u8>> {
        self.private_key.sign(request_data)
    }

    pub fn has_required_attributes(&self, required: &[String]) -> bool {
        required.iter().all(|attr| {
            if let Some((key, value)) = attr.split_once('=') {
                self.attributes
                    .get(key)
                    .map(|v| v == value)
                    .unwrap_or(false)
            } else {
                false
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_config_bundle::{AgentRole, BundleTarget};

    #[test]
    fn test_encrypt_bundle() {
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
        bundle.add_setting("timeout".to_string(), serde_json::json!(30));

        let encryptor = ConfigBundleEncryptor::new("https://kas.example.com".to_string());

        let policy = Policy {
            attributes: vec![PolicyAttribute {
                attribute: "agent.role".to_string(),
                display_name: "Agent Role".to_string(),
            }],
            dissemination: vec!["agent.role=test-agent".to_string()],
        };

        let encrypted = encryptor.encrypt_bundle(&bundle, policy).unwrap();

        assert_eq!(encrypted.bundle_id, bundle.bundle_id);
        assert!(!encrypted.encrypted_data.is_empty());
        assert_eq!(encrypted.policy_manifest.kas_url, "https://kas.example.com");
        assert_eq!(encrypted.policy_manifest.dissemination.len(), 1);
    }

    #[test]
    fn test_decrypt_requires_kas() {
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

        let policy = Policy {
            attributes: vec![PolicyAttribute {
                attribute: "agent.role".to_string(),
                display_name: "Agent Role".to_string(),
            }],
            dissemination: vec!["agent.role=test-agent".to_string()],
        };

        let encrypted = encryptor.encrypt_bundle(&bundle, policy).unwrap();

        let mut attributes = HashMap::new();
        attributes.insert("agent.role".to_string(), "test-agent".to_string());

        let identity = AgentIdentity::new("test-agent-001".to_string(), attributes).unwrap();
        let decryptor = ConfigBundleDecryptor::new(identity);

        let result = decryptor.decrypt_bundle(&encrypted);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("KAS integration"));
    }

    #[test]
    fn test_agent_identity_creation() {
        let mut attributes = HashMap::new();
        attributes.insert("agent.role".to_string(), "test-agent".to_string());

        let identity = AgentIdentity::new("test-agent-001".to_string(), attributes).unwrap();

        assert_eq!(identity.agent_id, "test-agent-001");
        assert!(identity.attributes.contains_key("agent.role"));
    }

    #[test]
    fn test_sign_request() {
        let mut attributes = HashMap::new();
        attributes.insert("agent.role".to_string(), "test-agent".to_string());

        let identity = AgentIdentity::new("test-agent-001".to_string(), attributes).unwrap();

        let request_data = b"test request data";
        let signature = identity.sign_request(request_data).unwrap();

        assert!(!signature.is_empty());
    }

    #[test]
    fn test_has_required_attributes() {
        let mut attributes = HashMap::new();
        attributes.insert("agent.role".to_string(), "test-agent".to_string());
        attributes.insert("environment".to_string(), "production".to_string());

        let identity = AgentIdentity::new("test-agent-001".to_string(), attributes).unwrap();

        assert!(identity.has_required_attributes(&[
            "agent.role=test-agent".to_string(),
            "environment=production".to_string()
        ]));

        assert!(!identity.has_required_attributes(&["agent.role=other-agent".to_string()]));
    }
}
