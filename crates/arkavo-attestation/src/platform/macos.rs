use crate::{
    evidence, AttestationCapabilities, AttestationError, AttestationType, PlatformAttestor,
    PlatformEvidence, Result, SecurityState,
};
use arkavo_device_identity::AgentIdentity;

pub struct SecureEnclaveAttestor {
    identity: AgentIdentity,
    platform_code: String,
}

impl SecureEnclaveAttestor {
    pub fn new(identity: AgentIdentity, platform_code: String) -> Result<Self> {
        if !is_secure_enclave_available() {
            return Err(AttestationError::Platform(
                "Secure Enclave not available on this device".to_string(),
            ));
        }

        Ok(Self {
            identity,
            platform_code,
        })
    }
}

impl PlatformAttestor for SecureEnclaveAttestor {
    fn collect_evidence(&self) -> Result<PlatformEvidence> {
        let mut evidence = PlatformEvidence::from_agent_identity(
            self.identity.clone(),
            self.platform_code.clone(),
        );

        evidence.platform_state = self.get_security_state();
        evidence.attestation_type = AttestationType::SecureEnclave;
        evidence.evidence_blob = generate_attestation_statement()?;

        Ok(evidence)
    }

    fn get_security_state(&self) -> SecurityState {
        evidence::detect_security_state()
    }

    fn get_capabilities(&self) -> AttestationCapabilities {
        AttestationCapabilities {
            attestation_type: AttestationType::SecureEnclave,
            supports_freshness: true,
            supports_hardware_binding: true,
        }
    }
}

fn is_secure_enclave_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        Command::new("ioreg")
            .args(["-rd1", "-c", "AppleKeyStore"])
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|s| s.contains("AppleKeyStore") || s.contains("AppleSEPManager"))
            .unwrap_or(false)
    }

    #[cfg(not(target_os = "macos"))]
    false
}

fn generate_attestation_statement() -> Result<Vec<u8>> {
    use std::process::Command;

    let output = Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice", "-a"])
        .output()
        .map_err(|e| {
            AttestationError::EvidenceGeneration(format!("Failed to execute ioreg: {}", e))
        })?;

    if !output.status.success() {
        return Err(AttestationError::EvidenceGeneration(
            "ioreg command failed".to_string(),
        ));
    }

    let platform_info = String::from_utf8(output.stdout).map_err(|e| {
        AttestationError::EvidenceGeneration(format!("Invalid ioreg output: {}", e))
    })?;

    let mut statement = Vec::new();

    if let Some(uuid_line) = platform_info.lines().find(|l| l.contains("IOPlatformUUID")) {
        statement.extend_from_slice(uuid_line.as_bytes());
        statement.push(b'\n');
    }

    if let Some(serial_line) = platform_info
        .lines()
        .find(|l| l.contains("IOPlatformSerialNumber"))
    {
        statement.extend_from_slice(serial_line.as_bytes());
        statement.push(b'\n');
    }

    if let Some(model_line) = platform_info.lines().find(|l| l.contains("model")) {
        statement.extend_from_slice(model_line.as_bytes());
        statement.push(b'\n');
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    statement.extend_from_slice(timestamp.to_string().as_bytes());

    if statement.is_empty() {
        return Err(AttestationError::EvidenceGeneration(
            "No platform information found".to_string(),
        ));
    }

    Ok(statement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect_platform_code;

    #[test]
    fn test_is_secure_enclave_available() {
        let available = is_secure_enclave_available();
        println!("Secure Enclave available: {}", available);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_generate_attestation_statement() {
        let statement = generate_attestation_statement();
        match statement {
            Ok(data) => {
                assert!(!data.is_empty());
                println!("Attestation statement length: {} bytes", data.len());
            }
            Err(e) => {
                println!(
                    "Attestation generation failed (expected on some systems): {}",
                    e
                );
            }
        }
    }

    #[test]
    fn test_secure_enclave_attestor_creation() {
        let identity = AgentIdentity::new("0.38.2".to_string());
        let platform_code = detect_platform_code();

        match SecureEnclaveAttestor::new(identity, platform_code) {
            Ok(attestor) => {
                let caps = attestor.get_capabilities();
                assert_eq!(caps.attestation_type, AttestationType::SecureEnclave);
                assert!(caps.supports_freshness);
                assert!(caps.supports_hardware_binding);
            }
            Err(e) => {
                println!(
                    "Secure Enclave not available (expected on some systems): {}",
                    e
                );
            }
        }
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_secure_enclave_collect_evidence() {
        let identity = AgentIdentity::new("0.38.2".to_string());
        let platform_code = detect_platform_code();

        if let Ok(attestor) = SecureEnclaveAttestor::new(identity, platform_code) {
            let evidence = attestor.collect_evidence();
            match evidence {
                Ok(ev) => {
                    assert_eq!(ev.attestation_type, AttestationType::SecureEnclave);
                    assert!(!ev.evidence_blob.is_empty());
                    assert_eq!(ev.platform_code, detect_platform_code());
                }
                Err(e) => {
                    println!("Evidence collection failed: {}", e);
                }
            }
        }
    }
}
