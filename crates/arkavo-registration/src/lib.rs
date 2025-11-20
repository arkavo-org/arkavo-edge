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
}

impl AgentDescriptor {
    pub fn new(
        public_key: AgentPublicKey,
        endpoint: String,
        mdns_service: Option<String>,
        agent_id_short_sha: String,
    ) -> Self {
        Self {
            public_key: public_key.to_base64(),
            endpoint,
            mdns_service,
            agent_id_short_sha,
        }
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
}
