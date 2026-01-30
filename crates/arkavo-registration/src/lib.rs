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
    /// Format: `arkavo://agent/authorize?did=...&name=...&rpc=ws://...&entitlements=...`
    ///
    /// The `rpc` parameter provides the WebSocket JSON-RPC endpoint for the mobile
    /// client to connect and use `registration.challenge` / `registration.verify`.
    ///
    /// # Panics
    /// Panics if the endpoint contains an unroutable address (0.0.0.0) or ephemeral port (0).
    pub fn to_authorization_url(&self) -> String {
        // Validate endpoint is routable before generating URL
        self.validate_endpoint();

        let did = self.did_key.as_deref().unwrap_or("");
        let mut url = format!("arkavo://agent/authorize?did={}", urlencoding::encode(did));

        if let Some(name) = &self.name {
            url.push_str(&format!("&name={}", urlencoding::encode(name)));
        }

        // Convert http:// endpoint to ws:// for WebSocket JSON-RPC connection
        let ws_endpoint = self.endpoint.replacen("http://", "ws://", 1);
        url.push_str(&format!("&rpc={}", urlencoding::encode(&ws_endpoint)));

        if !self.entitlements.is_empty() {
            let entitlements_str = self.entitlements.join(",");
            url.push_str(&format!(
                "&entitlements={}",
                urlencoding::encode(&entitlements_str)
            ));
        }

        url
    }

    /// Validate that the endpoint is routable by clients.
    ///
    /// # Panics
    /// Panics if:
    /// - The endpoint contains 0.0.0.0 (not routable - use actual IP)
    /// - The endpoint contains port 0 (ephemeral - use actual bound port)
    fn validate_endpoint(&self) {
        if self.endpoint.contains("0.0.0.0") {
            panic!(
                "Cannot generate authorization URL with unroutable address 0.0.0.0. \
                 Use the actual local IP address (e.g., 192.168.x.x). Endpoint: {}",
                self.endpoint
            );
        }

        if self.endpoint.ends_with(":0") || self.endpoint.contains(":0/") {
            panic!(
                "Cannot generate authorization URL with ephemeral port :0. \
                 Use the actual bound port after server starts. Endpoint: {}",
                self.endpoint
            );
        }
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

    #[test]
    fn test_authorization_url_contains_rpc_endpoint() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let descriptor = AgentDescriptor::new(
            public_key,
            "http://192.168.1.100:8342".to_string(),
            None,
            "test123".to_string(),
        )
        .with_name("test-agent")
        .with_entitlements(vec!["agent.capability.chat".to_string()]);

        let url = descriptor.to_authorization_url();

        // Must contain rpc parameter for WebSocket JSON-RPC endpoint
        assert!(
            url.contains("rpc="),
            "Authorization URL must contain rpc parameter for WebSocket endpoint. URL: {url}"
        );
        assert!(
            url.contains("192.168.1.100"),
            "Authorization URL must contain IP address. URL: {url}"
        );
        assert!(
            url.contains("8342"),
            "Authorization URL must contain port. URL: {url}"
        );
    }

    #[test]
    fn test_authorization_url_rpc_uses_websocket_scheme() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let descriptor = AgentDescriptor::new(
            public_key,
            "http://192.168.1.100:8342".to_string(),
            None,
            "ws123".to_string(),
        )
        .with_name("test-agent");

        let url = descriptor.to_authorization_url();

        // RPC endpoint should use ws:// scheme for WebSocket connection
        assert!(
            url.contains("rpc=ws%3A%2F%2F192.168.1.100%3A8342"),
            "RPC endpoint must use ws:// scheme. URL: {url}"
        );
    }

    #[test]
    #[should_panic(expected = "unroutable address 0.0.0.0")]
    fn test_authorization_url_rejects_unroutable_address() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();

        // 0.0.0.0 is not routable - clients can't connect to it
        let descriptor = AgentDescriptor::new(
            public_key,
            "http://0.0.0.0:8342".to_string(),
            None,
            "test".to_string(),
        );

        // This should panic because 0.0.0.0 is not routable
        let _ = descriptor.to_authorization_url();
    }

    #[test]
    #[should_panic(expected = "ephemeral port :0")]
    fn test_authorization_url_rejects_port_zero() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();

        // Port 0 means "OS assigns a port" - not valid for clients to connect to
        let descriptor = AgentDescriptor::new(
            public_key,
            "http://192.168.1.100:0".to_string(),
            None,
            "test".to_string(),
        );

        // This should panic because port 0 is ephemeral
        let _ = descriptor.to_authorization_url();
    }
}
