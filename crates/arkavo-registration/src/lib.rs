use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod qr;

#[derive(Error, Debug)]
pub enum RegistrationError {
    #[error("QR code generation failed: {0}")]
    QrCodeGeneration(String),
    #[error("Invalid payload: {0}")]
    InvalidPayload(String),
    #[error("Crypto error: {0}")]
    CryptoError(#[from] arkavo_crypto::CryptoError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDescriptor {
    pub public_key: String,
    pub endpoint: String,
    pub mdns_service: Option<String>,
    pub agent_id_short_sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub did_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entitlements: Vec<String>,
}

impl AgentDescriptor {
    pub fn new(
        public_key: AgentPublicKey,
        endpoint: String,
        mdns_service: Option<String>,
        agent_id_short_sha: String,
    ) -> Self {
        let did_key = Some(public_key.to_did_key());
        Self {
            public_key: public_key.to_base64(),
            endpoint,
            mdns_service,
            agent_id_short_sha,
            did_key,
            name: None,
            entitlements: Vec::new(),
        }
    }

    /// Set the agent name for authorization.
    #[must_use]
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the entitlements (capabilities) for authorization.
    #[must_use]
    pub fn with_entitlements(mut self, entitlements: Vec<String>) -> Self {
        self.entitlements = entitlements;
        self
    }

    /// Generate authorization URL for mobile app scanning.
    ///
    /// Format: `arkavo://agent/authorize?did=...&name=...&entitlements=...`
    pub fn to_authorization_url(&self) -> String {
        let did = self.did_key.as_deref().unwrap_or("");
        let mut url = format!("arkavo://agent/authorize?did={}", urlencoding::encode(did));

        if let Some(name) = &self.name {
            url.push_str(&format!("&name={}", urlencoding::encode(name)));
        }

        if !self.entitlements.is_empty() {
            let entitlements_str = self.entitlements.join(",");
            url.push_str(&format!(
                "&entitlements={}",
                urlencoding::encode(&entitlements_str)
            ));
        }

        url
    }

    /// Legacy URL format for backward compatibility.
    pub fn to_url(&self) -> String {
        let mut url = format!("arkavo://agent?public_key={}", self.public_key);

        if let Some(mdns) = &self.mdns_service {
            url.push_str(&format!("&mdns_service={}", urlencoding::encode(mdns)));
        }

        url
    }

    pub fn to_json(&self) -> Result<String, RegistrationError> {
        serde_json::to_string(self).map_err(|e| RegistrationError::InvalidPayload(e.to_string()))
    }

    pub fn from_json(json: &str) -> Result<Self, RegistrationError> {
        serde_json::from_str(json).map_err(|e| RegistrationError::InvalidPayload(e.to_string()))
    }

    pub fn public_key(&self) -> Result<AgentPublicKey, RegistrationError> {
        AgentPublicKey::from_base64(&self.public_key).map_err(|e| e.into())
    }
}

pub fn sign_challenge(challenge: &[u8], keypair: &AgentKeypair) -> Vec<u8> {
    keypair.sign(challenge)
}

pub fn verify_challenge(
    challenge: &[u8],
    signature: &[u8],
    public_key: &AgentPublicKey,
) -> Result<(), RegistrationError> {
    public_key
        .verify(challenge, signature)
        .map_err(|e| e.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_descriptor_serialization() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let descriptor = AgentDescriptor::new(
            public_key,
            "http://localhost:8342".to_string(),
            Some("arkavo-agent._tcp.local.".to_string()),
            "abc1234".to_string(),
        );

        let json = descriptor.to_json().unwrap();
        let restored = AgentDescriptor::from_json(&json).unwrap();

        assert_eq!(descriptor.public_key, restored.public_key);
        assert_eq!(descriptor.endpoint, restored.endpoint);
        assert_eq!(descriptor.mdns_service, restored.mdns_service);
        assert_eq!(descriptor.agent_id_short_sha, restored.agent_id_short_sha);
    }

    #[test]
    fn test_challenge_signing() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let challenge = b"random_challenge_data";

        let signature = sign_challenge(challenge, &keypair);
        assert!(verify_challenge(challenge, &signature, &public_key).is_ok());
    }

    #[test]
    fn test_challenge_verification_fails_on_wrong_message() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let challenge = b"random_challenge_data";
        let wrong_challenge = b"different_challenge";

        let signature = sign_challenge(challenge, &keypair);
        assert!(verify_challenge(wrong_challenge, &signature, &public_key).is_err());
    }

    #[test]
    fn test_public_key_extraction() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let descriptor = AgentDescriptor::new(
            public_key.clone(),
            "http://localhost:8342".to_string(),
            None,
            "test123".to_string(),
        );

        let extracted_key = descriptor.public_key().unwrap();
        assert_eq!(public_key.to_bytes(), extracted_key.to_bytes());
    }

    #[test]
    fn test_to_url() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let descriptor = AgentDescriptor::new(
            public_key.clone(),
            "http://localhost:8342".to_string(),
            Some("agent-name._tcp.local.".to_string()),
            "abc1234".to_string(),
        );

        let url = descriptor.to_url();
        assert!(url.starts_with("arkavo://agent?public_key="));
        assert!(url.contains("&mdns_service="));
        assert!(url.contains("agent-name._tcp.local."));
    }

    #[test]
    fn test_to_url_without_mdns() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let descriptor = AgentDescriptor::new(
            public_key,
            "http://localhost:8342".to_string(),
            None,
            "abc1234".to_string(),
        );

        let url = descriptor.to_url();
        assert!(url.starts_with("arkavo://agent?public_key="));
        assert!(!url.contains("&mdns_service="));
    }
}
