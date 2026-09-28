//! macOS Secure Enclave backend for the hardware attestation-key seam.
//!
//! Generates a non-extractable P-256 key inside the Secure Enclave via Apple's
//! Security.framework (`security-framework`), using Apple's native crypto — no
//! OpenSSL. The private scalar never leaves the enclave; only the public key is
//! exported, in SEC1 uncompressed form.
//!
//! Spec: specs/arkavo-edge/hardware-attestation.spec.yaml (HATT-001).

use crate::AttestationType;
use crate::attestation_key::{AssuranceTier, HardwareAttestationKey, HardwareKeystore};
use security_framework::access_control::SecAccessControl;
use security_framework::key::{GenerateKeyOptions, KeyType, SecKey, Token};
use security_framework_sys::access_control::kSecAccessControlPrivateKeyUsage;

/// Number of bytes in a SEC1 uncompressed P-256 public key (`0x04 || X || Y`).
const SEC1_UNCOMPRESSED_P256_LEN: usize = 65;

/// Keystore backed by the macOS Secure Enclave.
///
/// `generate()` requests a non-extractable EC (P-256) private key resident in
/// the Secure Enclave. This succeeds only on hardware with a Secure Enclave and
/// a binary entitled to use it (a signed binary); on any other configuration —
/// or on any Security.framework error — it returns `None` so the caller can
/// fall back to a software key.
pub struct SecureEnclaveKeystore;

impl SecureEnclaveKeystore {
    /// Attempt to mint a Secure-Enclave-resident P-256 key and export its
    /// public key as SEC1 uncompressed bytes. `None` on any failure.
    fn generate_public_key_sec1() -> Option<Vec<u8>> {
        // Require user presence is intentionally NOT set: this is an unattended
        // attestation key, gated only by private-key-usage so it can sign
        // without a passcode/biometric prompt. The protection mode is left at
        // the default (accessible when unlocked) since the key is non-permanent.
        let access_control =
            SecAccessControl::create_with_flags(kSecAccessControlPrivateKeyUsage).ok()?;

        let mut options = GenerateKeyOptions::default();
        options
            .set_key_type(KeyType::ec())
            .set_size_in_bits(256)
            .set_token(Token::SecureEnclave)
            .set_access_control(access_control);

        let private_key = SecKey::new(&options).ok()?;
        let public_key = private_key.public_key()?;
        let sec1 = public_key.external_representation()?.to_vec();

        // Security.framework returns an EC public key in ANSI X9.63 form, which
        // for P-256 is the 65-byte SEC1 uncompressed encoding `0x04 || X || Y`.
        if sec1.len() == SEC1_UNCOMPRESSED_P256_LEN && sec1.first() == Some(&0x04) {
            Some(sec1)
        } else {
            None
        }
    }
}

impl HardwareKeystore for SecureEnclaveKeystore {
    fn generate(&self) -> Option<HardwareAttestationKey> {
        let public_key_sec1 = Self::generate_public_key_sec1()?;
        Some(HardwareAttestationKey {
            public_key_sec1,
            attestation_type: AttestationType::SecureEnclave,
            tier: AssuranceTier::High,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_never_panics_and_is_graceful() {
        // On CI / unsigned binaries this returns None; the contract is that it
        // never panics regardless of Secure Enclave availability.
        let result = SecureEnclaveKeystore.generate();
        if let Some(hardware) = result {
            assert_eq!(hardware.attestation_type, AttestationType::SecureEnclave);
            assert_eq!(hardware.tier, AssuranceTier::High);
            assert_eq!(hardware.public_key_sec1.len(), SEC1_UNCOMPRESSED_P256_LEN);
            assert_eq!(hardware.public_key_sec1.first(), Some(&0x04));
        }
    }
}
