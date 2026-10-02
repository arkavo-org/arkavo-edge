use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod qr;

fn percent_encode(input: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

/// Most Unicode scalars the Arkavo app accepts in a link's `name`.
const LINK_NAME_MAX_CHARS: usize = 64;

/// The agent name as the authorization link may carry it, or `None` when
/// nothing printable is left.
///
/// The Arkavo app refuses the whole link, not just the name, when the name is
/// longer than 64 scalars or holds a control or format character. The default
/// name is `<hostname>-<folder>`, which easily runs past 64, so the name is
/// shortened here rather than leave a QR code that scans to nothing. Without
/// a name the app labels the agent itself; the DID stays the identity.
fn link_name(name: &str) -> Option<String> {
    use unicode_general_category::{GeneralCategory, get_general_category};
    let kept: String = name
        .chars()
        .filter(|&c| !c.is_control() && get_general_category(c) != GeneralCategory::Format)
        .take(LINK_NAME_MAX_CHARS)
        .collect();
    (!kept.trim().is_empty()).then_some(kept)
}

/// Entitlements an agent asks for when its link is scanned. They are
/// attribute FQNs because authnz-rs delegates from the person's own
/// entitlements, which are FQNs; it refuses names the person does not hold,
/// so the older `agent.capability.*` names could never be authorized.
pub const DEFAULT_ENTITLEMENTS: &[&str] = &[
    "https://arkavo.ai/attr/tdf/value/decrypt",
    "https://arkavo.ai/attr/action/value/read",
];

/// [`DEFAULT_ENTITLEMENTS`] as the owned list a descriptor takes.
pub fn default_entitlements() -> Vec<String> {
    DEFAULT_ENTITLEMENTS.iter().map(|e| e.to_string()).collect()
}

/// The agent's own identity keypair, created and stored on first use.
///
/// This is the key the authorization link names. It is kept apart from the
/// device keypair so that what a person authorizes is the agent, and so that
/// replacing one identity never silently changes the other.
pub fn load_or_create_agent_keypair() -> Result<AgentKeypair, RegistrationError> {
    use arkavo_device_identity::keypair;
    if let Some(bytes) = keypair::get_agent_keypair()? {
        return Ok(AgentKeypair::from_bytes(&bytes)?);
    }
    let created = AgentKeypair::generate();
    keypair::store_agent_keypair(&created.to_bytes())?;
    Ok(created)
}

#[derive(Error, Debug)]
pub enum RegistrationError {
    #[error("QR code generation failed: {0}")]
    QrCodeGeneration(String),
    #[error("Invalid payload: {0}")]
    InvalidPayload(String),
    #[error("Crypto error: {0}")]
    CryptoError(#[from] arkavo_crypto::CryptoError),
    #[error("Identity storage error: {0}")]
    Storage(#[from] arkavo_device_identity::DeviceIdentityError),
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

    /// Set the entitlements for authorization: attribute FQNs such as
    /// [`DEFAULT_ENTITLEMENTS`], which authnz-rs delegates from the entitlements
    /// the approving person holds.
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
    /// It is written only for an `http://` endpoint: anything else names nothing
    /// a client can dial (the first-run welcome has no listener yet), and the
    /// Arkavo app refuses the whole link when `rpc` is not a `ws://host:port` URL.
    ///
    /// # Panics
    /// Panics if the endpoint contains an unroutable address (0.0.0.0) or ephemeral port (0).
    pub fn to_authorization_url(&self) -> String {
        let did = self.did_key.as_deref().unwrap_or("");
        let mut url = format!("arkavo://agent/authorize?did={}", percent_encode(did));

        if let Some(name) = self.name.as_deref().and_then(link_name) {
            url.push_str(&format!("&name={}", percent_encode(&name)));
        }

        if let Some(authority) = self.endpoint.strip_prefix("http://") {
            self.validate_endpoint();
            url.push_str(&format!(
                "&rpc={}",
                percent_encode(&format!("ws://{authority}"))
            ));
        }

        if !self.entitlements.is_empty() {
            let entitlements_str = self.entitlements.join(",");
            url.push_str(&format!(
                "&entitlements={}",
                percent_encode(&entitlements_str)
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
            url.push_str(&format!("&mdns_service={}", percent_encode(mdns)));
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
    use arkavo_test_macros::spec;

    #[spec("QREG-005")]
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

    #[spec("QREG-004")]
    #[test]
    fn test_challenge_signing() {
        let keypair = AgentKeypair::generate();
        let public_key = keypair.public_key();
        let challenge = b"random_challenge_data";

        let signature = sign_challenge(challenge, &keypair);
        assert!(verify_challenge(challenge, &signature, &public_key).is_ok());
    }

    #[spec("QREG-004")]
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

    #[spec("QREG-001")]
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

    #[spec("QREG-001")]
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

    #[spec("QREG-001")]
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

    #[spec("QREG-001")]
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

    #[spec("QREG-001")]
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

    #[spec("QREG-001")]
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

    fn link_name_param(name: &str) -> Option<String> {
        let descriptor = AgentDescriptor::new(
            AgentKeypair::generate().public_key(),
            "http://127.0.0.1:8342".to_string(),
            None,
            "name".to_string(),
        )
        .with_name(name);
        let url = descriptor.to_authorization_url();
        let encoded = url.split('&').find_map(|pair| pair.strip_prefix("name="))?;
        let mut bytes = Vec::new();
        let mut rest = encoded.as_bytes();
        while let Some((&b, tail)) = rest.split_first() {
            if b == b'%' {
                let hex = std::str::from_utf8(&tail[..2]).unwrap();
                bytes.push(u8::from_str_radix(hex, 16).unwrap());
                rest = &tail[2..];
            } else {
                bytes.push(b);
                rest = tail;
            }
        }
        Some(String::from_utf8(bytes).unwrap())
    }

    /// The default name is `<hostname>-<folder>`; past 64 scalars the Arkavo
    /// app refused the whole link, so the QR code scanned to nothing.
    #[spec("QREG-001")]
    #[test]
    fn test_authorization_url_shortens_a_long_name_to_64_scalars() {
        let long =
            "Pauls-Mac-Mini-M6-my-company-monorepo-with-a-rather-long-descriptive-folder-name";
        let name = link_name_param(long).unwrap();
        assert_eq!(name.chars().count(), 64);
        assert!(long.starts_with(&name));

        let wide = "é".repeat(80);
        assert_eq!(link_name_param(&wide).unwrap(), "é".repeat(64));
    }

    #[spec("QREG-001")]
    #[test]
    fn test_authorization_url_keeps_a_name_within_the_limit_as_written() {
        let exact = "a".repeat(64);
        assert_eq!(link_name_param(&exact).unwrap(), exact);
        assert_eq!(link_name_param("my agent").unwrap(), "my agent");
    }

    /// The app refuses a link whose name holds a control (Cc) or format (Cf)
    /// character, bidirectional overrides included.
    #[spec("QREG-001")]
    #[test]
    fn test_authorization_url_drops_control_and_format_characters_from_the_name() {
        assert_eq!(
            link_name_param("host\u{7}-\u{202E}evil\u{200B}\u{FEFF}-dir\n").unwrap(),
            "host-evil-dir"
        );
    }

    #[spec("QREG-001")]
    #[test]
    fn test_authorization_url_omits_a_name_with_nothing_printable() {
        assert_eq!(link_name_param("\u{200B}\u{1}"), None);
        assert_eq!(link_name_param("   "), None);
    }

    /// The first-run welcome has no listener; an `rpc` that is not a
    /// `ws://host:port` URL made the Arkavo app refuse the whole link.
    #[spec("QREG-001")]
    #[test]
    fn test_authorization_url_leaves_out_rpc_without_an_http_endpoint() {
        for endpoint in ["", "host._a2a._tcp.local."] {
            let url = AgentDescriptor::new(
                AgentKeypair::generate().public_key(),
                endpoint.to_string(),
                None,
                "welcome".to_string(),
            )
            .with_entitlements(default_entitlements())
            .to_authorization_url();
            assert!(!url.contains("rpc="), "{url}");
            assert!(url.contains("&entitlements="), "{url}");
        }
    }

    /// authnz-rs refuses to delegate a name the person does not hold, and
    /// accounts hold attribute FQNs, never `agent.capability.*`.
    #[spec("QREG-003")]
    #[test]
    fn test_default_entitlements_are_attribute_fqns_the_app_accepts() {
        let names = default_entitlements();
        assert!((1..=16).contains(&names.len()));
        for name in &names {
            assert!(name.starts_with("https://arkavo.ai/attr/"), "{name}");
            assert!(name.len() <= 256, "{name}");
            assert!(!name.contains(','), "{name}");
            assert!(!name.chars().any(|c| c.is_whitespace() || c.is_control()));
        }
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(unique.len(), names.len());
    }
}
