//! HMAC operations with constant-time verification
//!
//! This module provides HMAC-SHA256 operations for TDF integrity verification.
//! Critically, it uses constant-time comparison to prevent timing attacks.

use crate::types::PayloadKey;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use thiserror::Error;
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum HmacError {
    #[error("HMAC initialization failed")]
    InitFailed,

    #[error("HMAC verification failed")]
    VerificationFailed,

    #[error("Base64 decode error: {0}")]
    Base64Error(#[from] base64::DecodeError),
}

/// Calculate HMAC-SHA256 over data
///
/// This is a low-level function. For TDF operations, use the specific
/// functions like `calculate_root_signature` or `calculate_policy_binding`.
pub fn calculate_hmac(key: &[u8], data: &[u8]) -> Result<Vec<u8>, HmacError> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| HmacError::InitFailed)?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

/// Calculate root signature from GMAC tags
///
/// The root signature is calculated as:
/// Base64(HMAC-SHA256(payloadKey, concat(gmac1, gmac2, ...)))
///
/// This matches the OpenTDF Go SDK implementation where:
/// - GMAC tags are concatenated as raw bytes (not base64)
/// - HMAC-SHA256 is calculated over the concatenated tags
/// - Result is base64 encoded for storage
pub fn calculate_root_signature(
    gmac_tags: &[Vec<u8>],
    payload_key: &PayloadKey,
) -> Result<String, HmacError> {
    // Concatenate all raw GMAC tags
    // Use Zeroizing to ensure cryptographic material is wiped from memory on drop
    let mut aggregate_hash = Zeroizing::new(Vec::with_capacity(gmac_tags.len() * 16));
    for tag in gmac_tags {
        aggregate_hash.extend_from_slice(tag);
    }

    // Calculate HMAC-SHA256 over concatenated tags
    let hmac_result = calculate_hmac(payload_key.as_slice(), &aggregate_hash)?;

    // Base64 encode for storage
    Ok(BASE64.encode(&hmac_result))
}

/// Verify root signature against GMAC tags using constant-time comparison
///
/// This validates the integrity of encrypted segments by:
/// 1. Concatenating all GMAC tags as raw bytes
/// 2. Calculating HMAC-SHA256 over concatenated tags using payload key
/// 3. Comparing result with stored root signature in constant time
///
/// Returns Ok(()) if signature is valid, Err otherwise.
///
/// # Security Note
///
/// This function uses `subtle::ConstantTimeEq` to prevent timing attacks.
/// Previous implementations used variable-time slice comparison which could
/// leak information about the signature through timing side-channels.
pub fn verify_root_signature(
    gmac_tags: &[Vec<u8>],
    payload_key: &PayloadKey,
    expected_sig_b64: &str,
) -> Result<(), HmacError> {
    // Concatenate all raw GMAC tags
    // Use Zeroizing to ensure cryptographic material is wiped from memory on drop
    let mut aggregate_hash = Zeroizing::new(Vec::with_capacity(gmac_tags.len() * 16));
    for tag in gmac_tags {
        aggregate_hash.extend_from_slice(tag);
    }

    // Calculate HMAC-SHA256 over concatenated tags
    let calculated = calculate_hmac(payload_key.as_slice(), &aggregate_hash)?;

    // Decode expected signature
    let expected = BASE64.decode(expected_sig_b64)?;

    // CRITICAL: Use constant-time comparison to prevent timing attacks
    if calculated.ct_eq(&expected).into() {
        Ok(())
    } else {
        Err(HmacError::VerificationFailed)
    }
}

/// Calculate policy binding using HMAC-SHA256
///
/// This matches the OpenTDF Go SDK format:
/// 1. Base64 encode the policy JSON
/// 2. HMAC-SHA256 the base64-encoded policy using the key
/// 3. Hex encode the HMAC result (32 bytes → 64 hex chars)
/// 4. Base64 encode the hex string for storage
pub fn calculate_policy_binding(policy_json: &str, key: &[u8]) -> Result<String, HmacError> {
    let policy_base64 = BASE64.encode(policy_json);

    // HMAC the base64-encoded policy
    let hmac_result = calculate_hmac(key, policy_base64.as_bytes())?;

    // Hex encode the HMAC result
    let hex_string = hex::encode(hmac_result);

    // Base64 encode the hex string
    Ok(BASE64.encode(hex_string.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_hmac() {
        let key = b"test_key_32_bytes_long_for_hmac!";
        let data = b"test data";

        let result = calculate_hmac(key, data).unwrap();
        assert_eq!(result.len(), 32); // SHA-256 output is 32 bytes
    }

    #[test]
    fn test_root_signature_roundtrip() {
        let key_bytes = [0u8; 32];
        let key = PayloadKey::from_slice(&key_bytes).unwrap();

        let gmac_tags = vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]];

        let signature = calculate_root_signature(&gmac_tags, &key).unwrap();
        let result = verify_root_signature(&gmac_tags, &key, &signature);

        assert!(result.is_ok());
    }

    #[test]
    fn test_root_signature_verification_fails_on_wrong_sig() {
        let key_bytes = [0u8; 32];
        let key = PayloadKey::from_slice(&key_bytes).unwrap();

        let gmac_tags = vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]];

        let wrong_sig = BASE64.encode(b"wrong_signature_exactly_32bytes!");

        let result = verify_root_signature(&gmac_tags, &key, &wrong_sig);
        assert!(result.is_err());
    }

    #[test]
    fn test_constant_time_verification() {
        // This test verifies that verification uses constant-time comparison
        // We can't easily test timing in a unit test, but we verify the function works
        let key_bytes = [0u8; 32];
        let key = PayloadKey::from_slice(&key_bytes).unwrap();

        let gmac_tags = vec![vec![1, 2, 3, 4]];
        let signature = calculate_root_signature(&gmac_tags, &key).unwrap();

        // Correct signature
        assert!(verify_root_signature(&gmac_tags, &key, &signature).is_ok());

        // Wrong signature (differs in first byte)
        let mut wrong_sig_bytes = BASE64.decode(&signature).unwrap();
        wrong_sig_bytes[0] ^= 1;
        let wrong_sig = BASE64.encode(&wrong_sig_bytes);
        assert!(verify_root_signature(&gmac_tags, &key, &wrong_sig).is_err());

        // Wrong signature (differs in last byte)
        let mut wrong_sig_bytes = BASE64.decode(&signature).unwrap();
        wrong_sig_bytes[31] ^= 1;
        let wrong_sig = BASE64.encode(&wrong_sig_bytes);
        assert!(verify_root_signature(&gmac_tags, &key, &wrong_sig).is_err());
    }

    #[test]
    fn test_policy_binding() {
        let policy = r#"{"body":{"dataAttributes":[]}}"#;
        let key = b"test_key_32_bytes_long_for_hmac!";

        let binding = calculate_policy_binding(policy, key).unwrap();

        // Should be base64 encoded
        assert!(BASE64.decode(&binding).is_ok());

        // Should be deterministic
        let binding2 = calculate_policy_binding(policy, key).unwrap();
        assert_eq!(binding, binding2);
    }

    #[test]
    fn test_constant_time_verification_all_positions() {
        // Tests that verification detects single-bit flips at any byte position
        // This validates constant-time comparison covers the entire signature
        let key_bytes = [0u8; 32];
        let key = PayloadKey::from_slice(&key_bytes).unwrap();

        let gmac_tags = vec![vec![1, 2, 3, 4], vec![5, 6, 7, 8]];
        let signature = calculate_root_signature(&gmac_tags, &key).unwrap();

        // Test positions: start (0), middle (15), end (31)
        for position in [0, 15, 31] {
            let mut wrong_sig_bytes = BASE64.decode(&signature).unwrap();
            wrong_sig_bytes[position] ^= 1; // Flip single bit
            let wrong_sig = BASE64.encode(&wrong_sig_bytes);
            assert!(
                verify_root_signature(&gmac_tags, &key, &wrong_sig).is_err(),
                "Should detect bit flip at position {}",
                position
            );
        }

        // Correct signature should still verify
        assert!(verify_root_signature(&gmac_tags, &key, &signature).is_ok());
    }

    #[test]
    fn test_root_signature_multiple_segments() {
        // Test verification with varied segment counts to ensure concatenation works
        let key_bytes = [42u8; 32];
        let key = PayloadKey::from_slice(&key_bytes).unwrap();

        // Test with 1, 5, and 10 segments
        for segment_count in [1, 5, 10] {
            let gmac_tags: Vec<Vec<u8>> = (0..segment_count)
                .map(|i| vec![i as u8; 16]) // 16-byte GMAC tags
                .collect();

            let signature = calculate_root_signature(&gmac_tags, &key).unwrap();
            assert!(
                verify_root_signature(&gmac_tags, &key, &signature).is_ok(),
                "Should verify with {} segments",
                segment_count
            );

            // Tamper with one segment - should fail
            let mut tampered_tags = gmac_tags.clone();
            tampered_tags[0][0] ^= 1;
            assert!(
                verify_root_signature(&tampered_tags, &key, &signature).is_err(),
                "Should detect tampering with {} segments",
                segment_count
            );
        }
    }

    #[test]
    fn test_policy_binding_deterministic_and_unique() {
        let key = b"test_key_32_bytes_long_for_hmac!";

        let policy1 = r#"{"uuid":"550e8400-e29b-41d4-a716-446655440000","body":{"attributes":[]}}"#;
        let policy2 = r#"{"uuid":"550e8400-e29b-41d4-a716-446655440001","body":{"attributes":[]}}"#;

        // Same policy/key should produce identical binding
        let binding1a = calculate_policy_binding(policy1, key).unwrap();
        let binding1b = calculate_policy_binding(policy1, key).unwrap();
        assert_eq!(
            binding1a, binding1b,
            "Same inputs should produce same binding"
        );

        // Different policies should produce different bindings
        let binding2 = calculate_policy_binding(policy2, key).unwrap();
        assert_ne!(
            binding1a, binding2,
            "Different policies should produce different bindings"
        );

        // Different keys should produce different bindings
        let key2 = b"different_key_32_bytes_long_abc!";
        let binding3 = calculate_policy_binding(policy1, key2).unwrap();
        assert_ne!(
            binding1a, binding3,
            "Different keys should produce different bindings"
        );
    }
}
