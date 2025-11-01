use crate::{
    evidence, AttestationCapabilities, AttestationType, PlatformAttestor, PlatformEvidence, Result,
    SecurityState,
};
use arkavo_device_identity::AgentIdentity;

#[cfg(target_os = "macos")]
pub mod macos;

pub struct FallbackAttestor {
    identity: AgentIdentity,
    platform_code: String,
}

impl FallbackAttestor {
    pub fn new(identity: AgentIdentity, platform_code: String) -> Self {
        Self {
            identity,
            platform_code,
        }
    }
}

impl PlatformAttestor for FallbackAttestor {
    fn collect_evidence(&self) -> Result<PlatformEvidence> {
        let mut evidence = PlatformEvidence::from_agent_identity(
            self.identity.clone(),
            self.platform_code.clone(),
        );

        evidence.platform_state = self.get_security_state();
        evidence.attestation_type = AttestationType::SoftwareFingerprint;
        evidence.evidence_blob = generate_software_fingerprint()?;

        Ok(evidence)
    }

    fn get_security_state(&self) -> SecurityState {
        evidence::detect_security_state()
    }

    fn get_capabilities(&self) -> AttestationCapabilities {
        AttestationCapabilities {
            attestation_type: AttestationType::SoftwareFingerprint,
            supports_freshness: false,
            supports_hardware_binding: false,
        }
    }
}

fn generate_software_fingerprint() -> Result<Vec<u8>> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();

    #[cfg(target_os = "macos")]
    {
        if let Ok(output) = std::process::Command::new("ioreg")
            .args(["-rd1", "-c", "IOPlatformExpertDevice"])
            .output()
        {
            output.stdout.hash(&mut hasher);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(machine_id) = std::fs::read_to_string("/etc/machine-id") {
            machine_id.hash(&mut hasher);
        }
        if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
            cpuinfo.hash(&mut hasher);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("wmic")
            .args(["csproduct", "get", "UUID"])
            .output()
        {
            output.stdout.hash(&mut hasher);
        }
    }

    let hash = hasher.finish();
    Ok(hash.to_le_bytes().to_vec())
}

pub fn create_attestor(
    identity: AgentIdentity,
    platform_code: String,
) -> Box<dyn PlatformAttestor> {
    #[cfg(target_os = "macos")]
    {
        if let Ok(se_attestor) =
            macos::SecureEnclaveAttestor::new(identity.clone(), platform_code.clone())
        {
            return Box::new(se_attestor);
        }
    }

    #[cfg(feature = "tpm")]
    {
        if let Ok(tpm_attestor) =
            crate::platform::tpm::TpmAttestor::new(identity.clone(), platform_code.clone())
        {
            return Box::new(tpm_attestor);
        }
    }

    Box::new(FallbackAttestor::new(identity, platform_code))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect_platform_code;

    #[test]
    fn test_fallback_attestor_creation() {
        let identity = AgentIdentity::new("0.38.2".to_string());
        let platform_code = detect_platform_code();
        let attestor = FallbackAttestor::new(identity, platform_code);

        let caps = attestor.get_capabilities();
        assert_eq!(caps.attestation_type, AttestationType::SoftwareFingerprint);
        assert!(!caps.supports_freshness);
        assert!(!caps.supports_hardware_binding);
    }

    #[test]
    fn test_fallback_attestor_collect_evidence() {
        let identity = AgentIdentity::new("0.38.2".to_string());
        let platform_code = detect_platform_code();
        let attestor = FallbackAttestor::new(identity, platform_code);

        let evidence = attestor
            .collect_evidence()
            .expect("Failed to collect evidence");
        assert_eq!(
            evidence.attestation_type,
            AttestationType::SoftwareFingerprint
        );
        assert!(!evidence.evidence_blob.is_empty());
    }

    #[test]
    fn test_software_fingerprint_generation() {
        let fp = generate_software_fingerprint().expect("Failed to generate fingerprint");
        assert!(!fp.is_empty());
        assert_eq!(fp.len(), 8);
    }

    #[test]
    fn test_create_attestor_selects_best_backend() {
        let identity = AgentIdentity::new("0.38.2".to_string());
        let platform_code = detect_platform_code();
        let attestor = create_attestor(identity, platform_code);

        let caps = attestor.get_capabilities();

        #[cfg(target_os = "macos")]
        {
            assert!(matches!(
                caps.attestation_type,
                AttestationType::SecureEnclave | AttestationType::SoftwareFingerprint
            ));
        }

        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(caps.attestation_type, AttestationType::SoftwareFingerprint);
        }
    }
}
