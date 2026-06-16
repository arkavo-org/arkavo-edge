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
    public_key_sec1: Vec<u8>,
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

    /// The P-256 `did:key` identifier for this attestation key's public key.
    pub fn did_key(&self) -> String {
        arkavo_crypto::P256VerifyingKey::from_sec1_bytes(&self.public_key_sec1)
            .expect("stored SEC1 bytes are a valid P-256 public key")
            .to_did_key()
    }
}

/// Provision an attestation key using the software fallback (no hardware
/// keystore available). The resulting key is never Trusted-eligible.
pub fn provision_software() -> AttestationKey {
    let keypair = arkavo_crypto::P256SigningKeypair::generate();
    AttestationKey {
        attestation_type: AttestationType::SoftwareFingerprint,
        hardware_binding: false,
        assurance_tier: AssuranceTier::None,
        public_key_sec1: keypair.public_key().to_sec1_bytes(),
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

    #[spec("HATT-001")]
    #[test]
    fn test_software_attestation_key_derives_p256_did_key() {
        let key = provision_software();
        let did = key.did_key();
        assert!(
            did.starts_with("did:key:zDn"),
            "expected P-256 did:key, got {did}"
        );
        assert!(arkavo_crypto::P256VerifyingKey::from_did_key(&did).is_ok());
    }
}
