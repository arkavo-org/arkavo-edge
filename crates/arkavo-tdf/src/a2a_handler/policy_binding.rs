//! Policy binding verification for the A2A KAS.
//!
//! A TDF binds its policy to its key with `HMAC-SHA256(DEK, policy)`, where
//! `policy` is the manifest's base64 policy string. A KAS that releases the
//! key without checking the binding lets anyone holding a wrapped key pair it
//! with a policy they satisfy, and so read data whose real policy denies them.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

use super::KasError;
use crate::types::PolicyBinding;

/// The binding algorithm TDF defines; there is no other.
const BINDING_ALG: &str = "HS256";

/// Check `binding` against the DEK and the base64 policy exactly as sent.
///
/// Accepts the spec form `Base64(MAC)`, which opentdf-rs 0.16 and OpenTDFKit 5
/// write, and the legacy `Base64(hex(MAC))` that earlier SDKs wrote, as
/// opentdf/platform#4081 does. `verify_slice` compares in constant time.
pub(super) fn verify(
    binding: &PolicyBinding,
    policy_base64: &str,
    dek: &[u8],
) -> Result<(), KasError> {
    if binding.alg != BINDING_ALG {
        return Err(invalid(format!(
            "unsupported binding algorithm {:?}",
            binding.alg
        )));
    }
    let decoded = STANDARD
        .decode(&binding.hash)
        .map_err(|_| invalid("binding hash is not base64".to_string()))?;
    let expected = match decoded.len() {
        32 => decoded,
        64 => hex::decode(&decoded)
            .map_err(|_| invalid("legacy binding hash is not hex".to_string()))?,
        n => {
            return Err(invalid(format!(
                "binding hash is {n} bytes; expected a 32-byte MAC or its 64 hex characters"
            )));
        }
    };

    let mut mac = Hmac::<Sha256>::new_from_slice(dek)
        .map_err(|e| KasError::CryptoError(format!("policy binding key: {e}")))?;
    mac.update(policy_base64.as_bytes());
    mac.verify_slice(&expected)
        .map_err(|_| invalid("binding does not match the policy and key".to_string()))
}

fn invalid(reason: String) -> KasError {
    KasError::PolicyBindingInvalid(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    // Known answer cross-checked with `openssl dgst -sha256 -hmac` over the
    // base64 policy, the same vector opentdf-rs 0.16 and OpenTDFKit 5.0.0 pin.
    const KEY: &[u8] = b"test_key_32_bytes_long_for_hmac!";
    const POLICY_B64: &str = "eyJib2R5Ijp7ImRhdGFBdHRyaWJ1dGVzIjpbXX19";
    const SPEC_BINDING: &str = "03yakXSzXkEvVt/Om8VVgFBWI3a+vYCxqtK4Ki80sbo=";
    const LEGACY_BINDING: &str =
        "ZDM3YzlhOTE3NGIzNWU0MTJmNTZkZmNlOWJjNTU1ODA1MDU2MjM3NmJlYmQ4MGIxYWFkMmI4MmEyZjM0YjFiYQ==";

    fn assert_invalid(result: Result<(), KasError>) {
        assert!(
            matches!(result, Err(KasError::PolicyBindingInvalid(_))),
            "expected PolicyBindingInvalid, got {result:?}"
        );
    }

    #[spec("TDFS-006")]
    #[test]
    fn accepts_the_spec_binding() {
        verify(&PolicyBinding::new(SPEC_BINDING), POLICY_B64, KEY).unwrap();
    }

    #[spec("TDFS-006")]
    #[test]
    fn accepts_the_legacy_hex_binding() {
        assert_eq!(LEGACY_BINDING.len(), 88);
        verify(&PolicyBinding::new(LEGACY_BINDING), POLICY_B64, KEY).unwrap();
    }

    #[spec("TDFS-006")]
    #[test]
    fn refuses_a_modified_policy() {
        let swapped = STANDARD.encode(r#"{"body":{"dataAttributes":[{"attribute":"x"}]}}"#);
        assert_invalid(verify(&PolicyBinding::new(SPEC_BINDING), &swapped, KEY));
        assert_invalid(verify(&PolicyBinding::new(LEGACY_BINDING), &swapped, KEY));
    }

    #[spec("TDFS-006")]
    #[test]
    fn refuses_a_binding_made_with_another_key() {
        let other_key = [7u8; 32];
        assert_invalid(verify(
            &PolicyBinding::new(SPEC_BINDING),
            POLICY_B64,
            &other_key,
        ));
    }

    #[spec("TDFS-006")]
    #[test]
    fn refuses_malformed_bindings() {
        for hash in [
            "",
            "not base64!",
            "dGVzdC1oYXNo", // a 9-byte placeholder, the kind the old check let through
            // 64 bytes that are not hex
            &STANDARD.encode([b'z'; 64]),
        ] {
            assert_invalid(verify(&PolicyBinding::new(hash), POLICY_B64, KEY));
        }
    }

    #[spec("TDFS-006")]
    #[test]
    fn refuses_an_algorithm_other_than_hs256() {
        let binding = PolicyBinding {
            alg: "GMAC".to_string(),
            hash: SPEC_BINDING.to_string(),
        };
        assert_invalid(verify(&binding, POLICY_B64, KEY));
    }
}
