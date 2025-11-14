use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

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
}

impl AgentIdentity {
    pub fn new(app_version: String) -> Self {
        Self {
            device_id: DeviceId::new(),
            app_version,
            agent_id: Some(Uuid::new_v4()),
        }
    }

    pub fn with_device_id(device_id: DeviceId, app_version: String) -> Self {
        Self {
            device_id,
            app_version,
            agent_id: Some(Uuid::new_v4()),
        }
    }

    pub fn device_only(device_id: DeviceId, app_version: String) -> Self {
        Self {
            device_id,
            app_version,
            agent_id: None,
        }
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
}
