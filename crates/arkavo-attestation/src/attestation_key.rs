//! Hardware-rooted attestation key provisioning and assurance tiers.
//!
//! Specs: specs/arkavo-edge/hardware-attestation.spec.yaml (HATT-*).

use crate::AttestationType;

/// Hardware assurance tier for an attestation key.
///
/// Maps to the assurance ladder in `hardware-attestation.spec.yaml`: `High`
/// (Secure Enclave / discrete TPM) is the only Trusted-eligible tier; `Medium`
/// (virtualized TPM) is capped below Trusted; `None` (software fingerprint) is
/// never Trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssuranceTier {
    /// Secure Enclave or discrete TPM: non-extractable, hardware-bound.
    High,
    /// Virtualized TPM: hardware-adjacent but not a discrete part.
    Medium,
    /// Software fingerprint: no hardware binding.
    None,
}

impl AssuranceTier {
    /// Whether a key at this tier may ever reach the `Trusted` security state.
    pub fn trusted_eligible(self) -> bool {
        matches!(self, AssuranceTier::High)
    }
}

/// An agent attestation identity key together with its hardware provenance.
///
/// The private key material is never exposed: there is no accessor that returns
/// the private scalar, modeling the non-extractable hardware key at the type
/// level.
#[derive(Debug, Clone)]
pub struct AttestationKey {
    attestation_type: AttestationType,
    hardware_binding: bool,
    assurance_tier: AssuranceTier,
}

impl AttestationKey {
    /// How the key was attested (Secure Enclave, TPM quote, or software).
    pub fn attestation_type(&self) -> AttestationType {
        self.attestation_type
    }

    /// Whether the key is bound to non-extractable hardware.
    pub fn hardware_binding(&self) -> bool {
        self.hardware_binding
    }

    /// The hardware assurance tier of the key.
    pub fn assurance_tier(&self) -> AssuranceTier {
        self.assurance_tier
    }
}

/// Provision an attestation key using the software fallback (no hardware
/// keystore available). The resulting key is never Trusted-eligible.
pub fn provision_software() -> AttestationKey {
    AttestationKey {
        attestation_type: AttestationType::SoftwareFingerprint,
        hardware_binding: false,
        assurance_tier: AssuranceTier::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("HATT-010")]
    #[test]
    fn test_software_attestation_key_is_never_trusted() {
        let key = provision_software();
        assert_eq!(key.attestation_type(), AttestationType::SoftwareFingerprint);
        assert!(!key.hardware_binding());
        assert_eq!(key.assurance_tier(), AssuranceTier::None);
        assert!(!key.assurance_tier().trusted_eligible());
    }
}
