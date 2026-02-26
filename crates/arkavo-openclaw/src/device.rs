use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use tracing::{debug, info};

/// Ed25519 device identity for OpenClaw gateway authentication.
///
/// The device ID is the SHA256 hex digest of the raw 32-byte public key.
/// Signatures use the `v2` payload format expected by the gateway.
pub struct DeviceIdentity {
    signing_key: SigningKey,
    device_id: String,
}

/// Parameters for signing an OpenClaw v2 challenge.
pub struct ChallengeSignParams<'a> {
    pub client_id: &'a str,
    pub client_mode: &'a str,
    pub role: &'a str,
    pub scopes: &'a [String],
    pub signed_at_ms: u64,
    pub token: &'a str,
    pub nonce: &'a str,
}

impl DeviceIdentity {
    /// Generate a new random Ed25519 keypair.
    pub fn generate() -> Self {
        let mut rng = rand::thread_rng();
        let signing_key = SigningKey::generate(&mut rng);
        let device_id = derive_device_id(&signing_key.verifying_key());
        info!("generated new device identity: {device_id}");
        Self {
            signing_key,
            device_id,
        }
    }

    /// Load identity from a directory, or generate and persist if missing.
    pub fn load_or_create(dir: &Path) -> Result<Self, DeviceError> {
        let key_path = dir.join("device-key.bin");
        if key_path.exists() {
            let bytes =
                std::fs::read(&key_path).map_err(|e| DeviceError::Io(format!("read key: {e}")))?;
            let key_bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| DeviceError::InvalidKey("expected 32 bytes".to_string()))?;
            let signing_key = SigningKey::from_bytes(&key_bytes);
            let device_id = derive_device_id(&signing_key.verifying_key());
            debug!("loaded device identity: {device_id}");
            Ok(Self {
                signing_key,
                device_id,
            })
        } else {
            let identity = Self::generate();
            std::fs::create_dir_all(dir)
                .map_err(|e| DeviceError::Io(format!("create dir: {e}")))?;
            std::fs::write(&key_path, identity.signing_key.to_bytes())
                .map_err(|e| DeviceError::Io(format!("write key: {e}")))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| DeviceError::Io(format!("set perms: {e}")))?;
            }
            info!("persisted new device identity to {}", key_path.display());
            Ok(identity)
        }
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Raw 32-byte public key encoded as base64url (no padding).
    pub fn public_key_base64url(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.signing_key.verifying_key().as_bytes())
    }

    /// Sign the OpenClaw v2 challenge payload.
    ///
    /// Payload format: `v2|<deviceId>|<clientId>|<clientMode>|<role>|<scopes>|<signedAt>|<token>|<nonce>`
    pub fn sign_challenge(&self, params: &ChallengeSignParams<'_>) -> SignedDevice {
        let scopes_str = params.scopes.join(",");
        let payload = format!(
            "v2|{}|{}|{}|{}|{scopes_str}|{}|{}|{}",
            self.device_id,
            params.client_id,
            params.client_mode,
            params.role,
            params.signed_at_ms,
            params.token,
            params.nonce,
        );
        let signature = self.signing_key.sign(payload.as_bytes());
        let signature_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());

        SignedDevice {
            id: self.device_id.clone(),
            public_key: self.public_key_base64url(),
            signature: signature_b64,
            signed_at: params.signed_at_ms,
            nonce: params.nonce.to_string(),
        }
    }

    /// Default identity directory: `~/.arkavo/openclaw/identity/`
    pub fn default_dir() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".arkavo")
            .join("openclaw")
            .join("identity")
    }
}

/// Signed device block ready to include in a connect request.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SignedDevice {
    pub id: String,
    #[serde(rename = "publicKey")]
    pub public_key: String,
    pub signature: String,
    #[serde(rename = "signedAt")]
    pub signed_at: u64,
    pub nonce: String,
}

/// Persisted device auth tokens received after pairing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeviceAuthStore {
    pub version: u32,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub tokens: std::collections::HashMap<String, StoredToken>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredToken {
    pub token: String,
    pub role: String,
    pub scopes: Vec<String>,
    #[serde(rename = "updatedAtMs")]
    pub updated_at_ms: u64,
}

impl DeviceAuthStore {
    pub fn load(dir: &Path) -> Option<Self> {
        let path = dir.join("device-auth.json");
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    pub fn save(&self, dir: &Path) -> Result<(), DeviceError> {
        std::fs::create_dir_all(dir).map_err(|e| DeviceError::Io(format!("create dir: {e}")))?;
        let path = dir.join("device-auth.json");
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| DeviceError::Io(format!("serialize: {e}")))?;
        std::fs::write(&path, json).map_err(|e| DeviceError::Io(format!("write: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| DeviceError::Io(format!("set perms: {e}")))?;
        }
        Ok(())
    }

    pub fn get_token(&self, role: &str) -> Option<&StoredToken> {
        self.tokens.get(role)
    }
}

fn derive_device_id(verifying_key: &VerifyingKey) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifying_key.as_bytes());
    hex::encode(hasher.finalize())
}

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("IO error: {0}")]
    Io(String),
    #[error("Invalid key: {0}")]
    InvalidKey(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_identity() {
        let id = DeviceIdentity::generate();
        assert_eq!(id.device_id().len(), 64); // SHA256 hex = 64 chars
        assert!(id.device_id().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn public_key_is_base64url() {
        let id = DeviceIdentity::generate();
        let pk = id.public_key_base64url();
        assert!(!pk.is_empty());
        let decoded = URL_SAFE_NO_PAD.decode(&pk).unwrap();
        assert_eq!(decoded.len(), 32); // Ed25519 public key = 32 bytes
    }

    #[test]
    fn device_id_is_sha256_of_public_key() {
        let id = DeviceIdentity::generate();
        let pk_bytes = URL_SAFE_NO_PAD.decode(id.public_key_base64url()).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&pk_bytes);
        let expected = hex::encode(hasher.finalize());
        assert_eq!(id.device_id(), expected);
    }

    #[test]
    fn sign_challenge_produces_valid_signature() {
        let id = DeviceIdentity::generate();
        let scopes = vec!["operator.admin".to_string()];
        let signed = id.sign_challenge(&ChallengeSignParams {
            client_id: "cli",
            client_mode: "cli",
            role: "operator",
            scopes: &scopes,
            signed_at_ms: 1_700_000_000_000,
            token: "",
            nonce: "test-nonce-uuid",
        });

        assert_eq!(signed.id, id.device_id());
        assert_eq!(signed.nonce, "test-nonce-uuid");
        assert_eq!(signed.signed_at, 1_700_000_000_000);

        // Verify the signature is valid
        let scopes_str = "operator.admin";
        let payload = format!(
            "v2|{}|cli|cli|operator|{scopes_str}|1700000000000||test-nonce-uuid",
            id.device_id()
        );
        let sig_bytes = URL_SAFE_NO_PAD.decode(&signed.signature).unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&sig_bytes).unwrap();
        let pk_bytes = URL_SAFE_NO_PAD.decode(&signed.public_key).unwrap();
        let vk = VerifyingKey::from_bytes(&pk_bytes.try_into().unwrap()).unwrap();
        vk.verify_strict(payload.as_bytes(), &sig).unwrap();
    }

    #[test]
    fn different_nonces_produce_different_signatures() {
        let id = DeviceIdentity::generate();
        let s1 = id.sign_challenge(&ChallengeSignParams {
            client_id: "cli",
            client_mode: "cli",
            role: "operator",
            scopes: &[],
            signed_at_ms: 1000,
            token: "",
            nonce: "nonce-1",
        });
        let s2 = id.sign_challenge(&ChallengeSignParams {
            client_id: "cli",
            client_mode: "cli",
            role: "operator",
            scopes: &[],
            signed_at_ms: 1000,
            token: "",
            nonce: "nonce-2",
        });
        assert_ne!(s1.signature, s2.signature);
    }

    #[test]
    fn load_or_create_persists_and_reloads() {
        let dir = std::env::temp_dir().join(format!("arkavo-test-{}", uuid::Uuid::new_v4()));
        let id1 = DeviceIdentity::load_or_create(&dir).unwrap();
        let id2 = DeviceIdentity::load_or_create(&dir).unwrap();
        assert_eq!(id1.device_id(), id2.device_id());
        assert_eq!(id1.public_key_base64url(), id2.public_key_base64url());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn auth_store_round_trip() {
        let dir = std::env::temp_dir().join(format!("arkavo-test-{}", uuid::Uuid::new_v4()));
        let store = DeviceAuthStore {
            version: 1,
            device_id: "abc123".to_string(),
            tokens: [(
                "operator".to_string(),
                StoredToken {
                    token: "tok_xyz".to_string(),
                    role: "operator".to_string(),
                    scopes: vec!["operator.admin".to_string()],
                    updated_at_ms: 1_700_000_000_000,
                },
            )]
            .into_iter()
            .collect(),
        };
        store.save(&dir).unwrap();
        let loaded = DeviceAuthStore::load(&dir).unwrap();
        assert_eq!(loaded.device_id, "abc123");
        assert_eq!(loaded.get_token("operator").unwrap().token, "tok_xyz");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
