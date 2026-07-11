use arkavo_device_identity::{AgentIdentity, DeviceId};
use serde::{Deserialize, Serialize};

pub mod evidence;
pub mod platform;

#[derive(Debug, thiserror::Error)]
pub enum AttestationError {
    #[error("Platform error: {0}")]
    Platform(String),
    #[error("Evidence generation failed: {0}")]
    EvidenceGeneration(String),
    #[error("Unsupported platform: {0}")]
    UnsupportedPlatform(String),
}

pub type Result<T> = std::result::Result<T, AttestationError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityState {
    Trusted,
    Suspicious,
    Compromised,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttestationType {
    TpmQuote,
    SecureEnclave,
    SoftwareFingerprint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformEvidence {
    pub device_id: DeviceId,
    pub platform_code: String,
    pub platform_state: SecurityState,
    pub attestation_type: AttestationType,
    pub evidence_blob: Vec<u8>,
    pub app_version: String,
    pub timestamp: i64,
}

impl PlatformEvidence {
    pub fn from_agent_identity(identity: AgentIdentity, platform_code: String) -> Self {
        Self {
            device_id: identity.device_id,
            platform_code,
            platform_state: SecurityState::Unknown,
            attestation_type: AttestationType::SoftwareFingerprint,
            evidence_blob: Vec::new(),
            app_version: identity.app_version,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("System time before Unix epoch")
                .as_secs() as i64,
        }
    }
}

pub trait PlatformAttestor {
    fn collect_evidence(&self) -> Result<PlatformEvidence>;
    fn get_security_state(&self) -> SecurityState;
    fn get_capabilities(&self) -> AttestationCapabilities;
}

#[derive(Debug, Clone)]
pub struct AttestationCapabilities {
    pub attestation_type: AttestationType,
    pub supports_freshness: bool,
    pub supports_hardware_binding: bool,
}

pub fn detect_platform_code() -> String {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "macos-arm64".to_string();

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "macos-x86_64".to_string();

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "linux-x86_64".to_string();

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    return "linux-aarch64".to_string();

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "windows-x86_64".to_string();

    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    const TEST_APP_VERSION: &str = "0.38.2";

    #[test]
    fn test_detect_platform_code() {
        let platform = detect_platform_code();
        assert!(!platform.is_empty());
        assert_ne!(platform, "unknown");
    }

    #[test]
    fn test_security_state_serialization() {
        let state = SecurityState::Trusted;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"trusted\"");
    }

    #[test]
    fn test_attestation_type_serialization() {
        let at_type = AttestationType::TpmQuote;
        let json = serde_json::to_string(&at_type).unwrap();
        assert_eq!(json, "\"tpm_quote\"");
    }

    #[test]
    #[spec("ATT-002")]
    fn test_platform_evidence_from_agent_identity() {
        let identity = AgentIdentity::new(TEST_APP_VERSION.to_string());
        let platform_code = detect_platform_code();
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let evidence =
            PlatformEvidence::from_agent_identity(identity.clone(), platform_code.clone());

        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        assert_eq!(evidence.device_id, identity.device_id, "Device ID captured");
        assert_eq!(evidence.platform_code, platform_code);
        assert_eq!(evidence.platform_state, SecurityState::Unknown);
        assert_eq!(
            evidence.attestation_type,
            AttestationType::SoftwareFingerprint
        );
        assert!(
            evidence.evidence_blob.is_empty(),
            "Evidence blob initialized empty"
        );
        assert_eq!(evidence.app_version, TEST_APP_VERSION);
        assert!(
            evidence.timestamp >= before && evidence.timestamp <= after,
            "Timestamp recorded"
        );
    }
}
