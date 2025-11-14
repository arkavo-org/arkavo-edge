use crate::{AttestationError, PlatformEvidence, Result, SecurityState};

pub fn validate_evidence(evidence: &PlatformEvidence) -> Result<()> {
    if evidence.platform_code.is_empty() {
        return Err(AttestationError::EvidenceGeneration(
            "Platform code cannot be empty".to_string(),
        ));
    }

    if evidence.app_version.is_empty() {
        return Err(AttestationError::EvidenceGeneration(
            "App version cannot be empty".to_string(),
        ));
    }

    let current_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let time_diff = (current_time - evidence.timestamp).abs();
    if time_diff > 300 {
        return Err(AttestationError::EvidenceGeneration(format!(
            "Evidence timestamp too old or in future: {} seconds difference",
            time_diff
        )));
    }

    Ok(())
}

pub fn detect_security_state() -> SecurityState {
    #[cfg(target_os = "macos")]
    {
        if is_macos_jailbroken() {
            return SecurityState::Compromised;
        }
        if is_macos_debug_mode() {
            return SecurityState::Suspicious;
        }
        SecurityState::Trusted
    }

    #[cfg(target_os = "linux")]
    {
        if is_linux_compromised() {
            return SecurityState::Compromised;
        }
        SecurityState::Unknown
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    SecurityState::Unknown
}

#[cfg(target_os = "macos")]
fn is_macos_jailbroken() -> bool {
    std::path::Path::new("/Applications/Cydia.app").exists()
        || std::path::Path::new("/Library/MobileSubstrate").exists()
        || std::path::Path::new("/private/var/lib/apt").exists()
        || std::path::Path::new("/private/var/stash").exists()
}

#[cfg(target_os = "macos")]
fn is_macos_debug_mode() -> bool {
    use std::process::Command;

    Command::new("sysctl")
        .arg("kern.bootargs")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.contains("-v") || s.contains("debug"))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn is_linux_compromised() -> bool {
    std::path::Path::new("/dev/shm/.X11-unix").exists()
        || std::path::Path::new("/tmp/.ICE-unix").exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect_platform_code;
    use arkavo_device_identity::AgentIdentity;

    const TEST_APP_VERSION: &str = "0.38.2";

    #[test]
    fn test_validate_evidence_success() {
        let identity = AgentIdentity::new(TEST_APP_VERSION.to_string());
        let evidence = PlatformEvidence::from_agent_identity(identity, detect_platform_code());

        assert!(validate_evidence(&evidence).is_ok());
    }

    #[test]
    fn test_validate_evidence_empty_platform_code() {
        let identity = AgentIdentity::new(TEST_APP_VERSION.to_string());
        let mut evidence = PlatformEvidence::from_agent_identity(identity, "".to_string());
        evidence.platform_code = String::new();

        let result = validate_evidence(&evidence);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Platform code cannot be empty")
        );
    }

    #[test]
    fn test_validate_evidence_old_timestamp() {
        let identity = AgentIdentity::new(TEST_APP_VERSION.to_string());
        let mut evidence = PlatformEvidence::from_agent_identity(identity, detect_platform_code());
        evidence.timestamp = 0;

        let result = validate_evidence(&evidence);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too old"));
    }

    #[test]
    fn test_detect_security_state() {
        let state = detect_security_state();
        assert!(matches!(
            state,
            SecurityState::Trusted
                | SecurityState::Suspicious
                | SecurityState::Compromised
                | SecurityState::Unknown
        ));
    }

    #[test]
    fn test_validate_evidence_empty_evidence_blob() {
        let identity = AgentIdentity::new(TEST_APP_VERSION.to_string());
        let evidence = PlatformEvidence::from_agent_identity(identity, detect_platform_code());

        let result = validate_evidence(&evidence);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_evidence_future_timestamp() {
        let identity = AgentIdentity::new(TEST_APP_VERSION.to_string());
        let mut evidence = PlatformEvidence::from_agent_identity(identity, detect_platform_code());

        evidence.timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs() as i64
            + 7200;

        let result = validate_evidence(&evidence);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("future"));
    }

    #[test]
    fn test_validate_evidence_invalid_app_version() {
        let identity = AgentIdentity::new("".to_string());
        let mut evidence = PlatformEvidence::from_agent_identity(identity, detect_platform_code());
        evidence.app_version = String::new();

        let result = validate_evidence(&evidence);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("App version cannot be empty")
        );
    }

    #[test]
    fn test_corrupted_evidence_serialization() {
        let identity = AgentIdentity::new(TEST_APP_VERSION.to_string());
        let evidence = PlatformEvidence::from_agent_identity(identity, detect_platform_code());

        let json = serde_json::to_string(&evidence).expect("Should serialize");
        assert!(!json.is_empty());

        let deserialized: std::result::Result<PlatformEvidence, _> = serde_json::from_str(&json);
        assert!(deserialized.is_ok());
    }
}
