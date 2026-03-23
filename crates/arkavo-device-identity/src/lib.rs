use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

pub mod keypair;
pub mod storage;

#[derive(Debug, thiserror::Error)]
pub enum DeviceIdentityError {
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Invalid device ID format: {0}")]
    InvalidFormat(String),
    #[error("Platform not supported: {0}")]
    UnsupportedPlatform(String),
}

pub type Result<T> = std::result::Result<T, DeviceIdentityError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceId([u8; 16]);

impl DeviceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().into_bytes())
    }

    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub fn to_uuid(&self) -> Uuid {
        Uuid::from_bytes(self.0)
    }
}

impl Default for DeviceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_uuid())
    }
}

impl From<Uuid> for DeviceId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid.into_bytes())
    }
}

impl From<DeviceId> for Uuid {
    fn from(device_id: DeviceId) -> Self {
        Uuid::from_bytes(device_id.0)
    }
}

pub fn get_or_create_device_id() -> Result<DeviceId> {
    storage::get_or_create()
}

pub fn get_device_id() -> Result<Option<DeviceId>> {
    storage::get()
}

pub fn store_device_id(device_id: DeviceId) -> Result<()> {
    storage::store(device_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub device_id: DeviceId,
    pub app_version: String,
    pub agent_id: Option<Uuid>,
    /// DID:key identifier derived from the device public key.
    /// Set by the caller after key generation via `AgentPublicKey::to_did_key()`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub did: Option<String>,
}

impl AgentIdentity {
    pub fn new(app_version: String) -> Self {
        Self {
            device_id: DeviceId::new(),
            app_version,
            agent_id: Some(Uuid::new_v4()),
            did: None,
        }
    }

    pub fn with_device_id(device_id: DeviceId, app_version: String) -> Self {
        Self {
            device_id,
            app_version,
            agent_id: Some(Uuid::new_v4()),
            did: None,
        }
    }

    pub fn device_only(device_id: DeviceId, app_version: String) -> Self {
        Self {
            device_id,
            app_version,
            agent_id: None,
            did: None,
        }
    }

    /// Set the DID:key identifier derived from the device public key.
    pub fn with_did(mut self, did: String) -> Self {
        self.did = Some(did);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_id_creation() {
        let id1 = DeviceId::new();
        let id2 = DeviceId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_device_id_roundtrip() {
        let original = DeviceId::new();
        let bytes = *original.as_bytes();
        let restored = DeviceId::from_bytes(bytes);
        assert_eq!(original, restored);
    }

    #[test]
    fn test_device_id_uuid_conversion() {
        let uuid = Uuid::new_v4();
        let device_id = DeviceId::from(uuid);
        let uuid_back: Uuid = device_id.into();
        assert_eq!(uuid, uuid_back);
    }

    #[test]
    fn test_device_id_display() {
        let device_id = DeviceId::new();
        let display = format!("{}", device_id);
        assert!(display.len() == 36);
        assert!(display.contains('-'));
    }

    #[test]
    fn test_agent_identity_with_did() {
        let identity =
            AgentIdentity::new("1.0.0".to_string()).with_did("did:key:z6MkTest123".to_string());
        assert_eq!(identity.did, Some("did:key:z6MkTest123".to_string()));
    }

    #[test]
    fn test_agent_identity_did_defaults_none() {
        let identity = AgentIdentity::new("1.0.0".to_string());
        assert_eq!(identity.did, None);
    }

    #[test]
    fn test_agent_identity_did_serialization_roundtrip() {
        let identity =
            AgentIdentity::new("1.0.0".to_string()).with_did("did:key:z6MkTest123".to_string());
        let json = serde_json::to_string(&identity).unwrap();
        let deserialized: AgentIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.did, Some("did:key:z6MkTest123".to_string()));
        assert_eq!(deserialized.device_id, identity.device_id);
    }

    #[test]
    fn test_agent_identity_without_did_serialization_roundtrip() {
        let identity = AgentIdentity::new("1.0.0".to_string());
        let json = serde_json::to_string(&identity).unwrap();
        assert!(!json.contains("did"));
        let deserialized: AgentIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.did, None);
    }
}
