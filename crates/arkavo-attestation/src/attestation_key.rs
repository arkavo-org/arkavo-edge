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

/// A platform keystore that can generate a non-extractable hardware attestation key.
pub trait HardwareKeystore {
    /// Generate a non-extractable key, or None if unavailable / generation fails on this device.
    fn generate(&self) -> Option<HardwareAttestationKey>;
}

/// Successful hardware key generation result.
pub struct HardwareAttestationKey {
    /// SEC1-encoded public key of the generated hardware key.
    pub public_key_sec1: Vec<u8>,
    /// How the key is attested (Secure Enclave, TPM quote, ...).
    pub attestation_type: AttestationType,
    /// The hardware assurance tier this keystore provides.
    pub tier: AssuranceTier,
}

/// Provision by trying each keystore in order; fall back to software if none succeed.
pub fn provision_with(keystores: &[&dyn HardwareKeystore]) -> AttestationKey {
    for keystore in keystores {
        if let Some(hardware) = keystore.generate() {
            return AttestationKey {
                attestation_type: hardware.attestation_type,
                hardware_binding: true,
                assurance_tier: hardware.tier,
                public_key_sec1: hardware.public_key_sec1,
            };
        }
    }
    provision_software()
}

/// Provision an attestation key using the best hardware keystore available on
/// this platform, falling back to software when no hardware-bound key can be
/// generated (no Secure Enclave, unsigned binary, or API failure).
#[cfg(target_os = "macos")]
pub fn provision() -> AttestationKey {
    provision_with(&[&crate::secure_enclave::SecureEnclaveKeystore])
}

/// Provision an attestation key. No hardware keystore is wired on this platform
/// yet, so this falls back to software.
#[cfg(not(target_os = "macos"))]
pub fn provision() -> AttestationKey {
    provision_with(&[])
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

/// The result of rotating an attestation identity after key loss or compromise.
#[derive(Debug, Clone)]
pub struct Rotation {
    /// The freshly minted attestation key that supersedes the previous one.
    pub new_key: AttestationKey,
    /// The `did:key` of the previous key, now revoked.
    pub revoked_did_key: String,
}

/// Rotate an attestation identity: mint a fresh key and revoke the previous one.
pub fn rotate(previous: &AttestationKey) -> Rotation {
    Rotation {
        new_key: provision(),
        revoked_did_key: previous.did_key(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("HATT-012")]
    #[test]
    fn test_rotation_mints_new_key_and_revokes_previous() {
        let old = provision_software();
        let rotation = rotate(&old);
        assert_ne!(
            rotation.new_key.did_key(),
            old.did_key(),
            "rotation must mint a fresh key with a different did:key"
        );
        assert_eq!(
            rotation.revoked_did_key,
            old.did_key(),
            "rotation must revoke the previous key's did:key"
        );
    }

    struct FakeVtpm;

    impl HardwareKeystore for FakeVtpm {
        fn generate(&self) -> Option<HardwareAttestationKey> {
            let public_key_sec1 = arkavo_crypto::P256SigningKeypair::generate()
                .public_key()
                .to_sec1_bytes();
            Some(HardwareAttestationKey {
                public_key_sec1,
                attestation_type: AttestationType::TpmQuote,
                tier: AssuranceTier::Medium,
            })
        }
    }

    #[spec("HATT-013")]
    #[test]
    fn test_vtpm_medium_tier_is_capped_below_trusted() {
        // PINS existing seam behavior: a virtualized-TPM keystore reports a
        // hardware binding at the Medium tier, and the assurance ladder caps
        // Medium below Trusted. vTPM detection itself lives in the deferred TPM
        // backend; this characterizes the cap the seam already enforces.
        let key = provision_with(&[&FakeVtpm]);
        assert_eq!(key.assurance_tier(), AssuranceTier::Medium);
        assert!(key.hardware_binding());
        assert!(!key.assurance_tier().trusted_eligible());
    }

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

    #[test]
    fn test_provision_with_no_keystores_falls_back_to_software() {
        let key = provision_with(&[]);
        assert_eq!(key.attestation_type(), AttestationType::SoftwareFingerprint);
        assert!(!key.hardware_binding());
        assert_eq!(key.assurance_tier(), AssuranceTier::None);
        assert!(key.did_key().starts_with("did:key:zDn"));
    }

    struct FakeSecureEnclave;

    impl HardwareKeystore for FakeSecureEnclave {
        fn generate(&self) -> Option<HardwareAttestationKey> {
            let public_key_sec1 = arkavo_crypto::P256SigningKeypair::generate()
                .public_key()
                .to_sec1_bytes();
            Some(HardwareAttestationKey {
                public_key_sec1,
                attestation_type: AttestationType::SecureEnclave,
                tier: AssuranceTier::High,
            })
        }
    }

    #[test]
    fn test_provision_with_available_keystore_uses_hardware() {
        let keystore = FakeSecureEnclave;
        let key = provision_with(&[&keystore]);
        assert!(key.hardware_binding());
        assert_eq!(key.assurance_tier(), AssuranceTier::High);
        assert_eq!(key.attestation_type(), AttestationType::SecureEnclave);
    }

    #[spec("HATT-001")]
    #[test]
    fn test_provision_yields_consistent_attestation_key() {
        // Runs everywhere: on CI / unsigned binaries this falls back to
        // software; on signed hardware with a Secure Enclave it yields a
        // hardware-bound key. Either outcome must satisfy the tier invariant.
        let key = provision();
        assert!(
            key.did_key().starts_with("did:key:zDn"),
            "expected P-256 did:key, got {}",
            key.did_key()
        );
        match key.attestation_type() {
            AttestationType::SecureEnclave => {
                assert!(key.hardware_binding());
                assert_eq!(key.assurance_tier(), AssuranceTier::High);
            }
            AttestationType::SoftwareFingerprint => {
                assert!(!key.hardware_binding());
                assert_eq!(key.assurance_tier(), AssuranceTier::None);
            }
            AttestationType::TpmQuote => {
                panic!("provision() never yields a TPM-quoted key on this platform");
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[spec("HATT-001")]
    #[test]
    #[ignore = "requires a signed macOS binary with Secure Enclave access"]
    fn test_secure_enclave_generates_hardware_bound_key() {
        use crate::secure_enclave::SecureEnclaveKeystore;

        // Documents the hardware residency contract. On a signed binary with a
        // Secure Enclave this returns Some and the key is hardware-bound; run
        // manually on real hardware.
        if let Some(hardware) = SecureEnclaveKeystore.generate() {
            assert_eq!(hardware.attestation_type, AttestationType::SecureEnclave);
            assert_eq!(hardware.tier, AssuranceTier::High);
            let key = provision_with(&[&SecureEnclaveKeystore]);
            assert!(key.hardware_binding());
            assert!(key.did_key().starts_with("did:key:zDn"));
        }
    }
}
