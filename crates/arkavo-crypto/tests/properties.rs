//! Property-based tests for cryptographic primitives
//!
//! These tests verify algebraic properties that must always hold:
//! - CRYPTO-INV-001: Signatures are deterministic for same key+message
//! - CRYPTO-INV-002: Key serialization round-trips correctly
//! - CRYPTO-INV-003: ECDH produces consistent shared secrets
//! - CRYPTO-INV-004: DID:key format is reversible

use arkavo_crypto::{AgentKeypair, AgentPublicKey, KasEcKeypair, KasEcPublicKey};
use proptest::prelude::*;

/// Strategy for generating valid message bytes
fn message_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..1024)
}

/// Strategy for generating valid 32-byte seeds
fn seed_strategy() -> impl Strategy<Value = [u8; 32]> {
    any::<[u8; 32]>()
}

proptest! {
    // CRYPTO-INV-001: Signature determinism property
    // Signing the same message with the same key produces the same signature
    #[test]
    fn prop_signature_determinism(seed in seed_strategy(), message in message_strategy()) {
        let keypair1 = AgentKeypair::from_bytes(&seed).unwrap();
        let keypair2 = AgentKeypair::from_bytes(&seed).unwrap();

        let sig1 = keypair1.sign(&message);
        let sig2 = keypair2.sign(&message);

        prop_assert_eq!(sig1, sig2, "Same seed should produce identical signatures");
    }

    // CRYPTO-INV-002: Key serialization round-trip property
    // Converting key to bytes and back preserves functionality
    #[test]
    fn prop_key_serialization_roundtrip(seed in seed_strategy()) {
        let original = AgentKeypair::from_bytes(&seed).unwrap();
        let bytes = original.to_bytes();
        let restored = AgentKeypair::from_bytes(&bytes).unwrap();

        let message = b"test message";
        let sig_original = original.sign(message);
        let sig_restored = restored.sign(message);

        prop_assert_eq!(
            sig_original, sig_restored,
            "Restored key should produce same signatures"
        );
    }

    // CRYPTO-INV-003: Public key derivation consistency
    // Same keypair always produces same public key
    #[test]
    fn prop_public_key_consistency(seed in seed_strategy()) {
        let keypair1 = AgentKeypair::from_bytes(&seed).unwrap();
        let keypair2 = AgentKeypair::from_bytes(&seed).unwrap();

        let pk1 = keypair1.public_key();
        let pk2 = keypair2.public_key();

        prop_assert_eq!(
            pk1.to_bytes(), pk2.to_bytes(),
            "Same seed should produce identical public keys"
        );
    }

    // CRYPTO-INV-004: Signature verification property
    // Valid signatures must verify correctly
    #[test]
    fn prop_signature_verifies(seed in seed_strategy(), message in message_strategy()) {
        let keypair = AgentKeypair::from_bytes(&seed).unwrap();
        let public_key = keypair.public_key();
        let signature = keypair.sign(&message);

        prop_assert!(
            public_key.verify(&message, &signature).is_ok(),
            "Valid signature should verify successfully"
        );
    }

    // CRYPTO-INV-005: Tampered message detection
    // Signatures don't verify for different messages
    #[test]
    fn prop_tampered_message_fails(
        seed in seed_strategy(),
        message1 in message_strategy(),
        message2 in message_strategy()
    ) {
        prop_assume!(!message1.is_empty() || !message2.is_empty());
        prop_assume!(message1 != message2);

        let keypair = AgentKeypair::from_bytes(&seed).unwrap();
        let public_key = keypair.public_key();
        let signature = keypair.sign(&message1);

        prop_assert!(
            public_key.verify(&message2, &signature).is_err(),
            "Signature for different message should fail verification"
        );
    }

    // CRYPTO-INV-006: DID:key round-trip property
    // DID encoding is reversible
    #[test]
    fn prop_did_key_roundtrip(seed in seed_strategy()) {
        let keypair = AgentKeypair::from_bytes(&seed).unwrap();
        let public_key = keypair.public_key();

        let did = public_key.to_did_key();
        let restored = AgentPublicKey::from_did_key(&did).unwrap();

        prop_assert_eq!(
            public_key.to_bytes(),
            restored.to_bytes(),
            "DID:key round-trip should preserve public key"
        );
    }

    // CRYPTO-INV-007: Base64 round-trip property
    #[test]
    fn prop_base64_roundtrip(seed in seed_strategy()) {
        let keypair = AgentKeypair::from_bytes(&seed).unwrap();
        let public_key = keypair.public_key();

        let base64 = public_key.to_base64();
        let restored = AgentPublicKey::from_base64(&base64).unwrap();

        prop_assert_eq!(
            public_key.to_bytes(),
            restored.to_bytes(),
            "Base64 round-trip should preserve public key"
        );
    }
}

// Property tests for EC P-256 (KAS operations)
proptest! {
    // CRYPTO-INV-011: ECDH shared secret symmetry
    // Both parties derive the same shared secret
    #[test]
    fn prop_ecdh_symmetry(seed1 in seed_strategy(), seed2 in seed_strategy()) {
        prop_assume!(seed1 != seed2);

        let keypair_a = KasEcKeypair::from_secret_bytes(&seed1).unwrap();
        let keypair_b = KasEcKeypair::from_secret_bytes(&seed2).unwrap();

        let shared_a = keypair_a.diffie_hellman(&keypair_b.public_key());
        let shared_b = keypair_b.diffie_hellman(&keypair_a.public_key());

        prop_assert_eq!(
            shared_a.clone(), shared_b,
            "ECDH should produce same shared secret for both parties"
        );
        prop_assert_eq!(
            shared_a.len(), 32,
            "Shared secret should be 32 bytes"
        );
    }

    // CRYPTO-INV-012: KAS key serialization round-trip
    #[test]
    fn prop_kas_key_roundtrip(seed in seed_strategy()) {
        let original = KasEcKeypair::from_secret_bytes(&seed).unwrap();
        let bytes = original.secret_bytes();
        let restored = KasEcKeypair::from_secret_bytes(&bytes).unwrap();

        let pk_original = original.public_key_sec1();
        let pk_restored = restored.public_key_sec1();

        prop_assert_eq!(
            pk_original, pk_restored,
            "KAS key round-trip should preserve public key"
        );
    }

    // CRYPTO-INV-013: SEC1 encoding round-trip
    #[test]
    fn prop_sec1_roundtrip(seed in seed_strategy()) {
        let keypair = KasEcKeypair::from_secret_bytes(&seed).unwrap();
        let public_key = keypair.public_key();

        let sec1_bytes = public_key.to_sec1_bytes();
        let restored = KasEcPublicKey::from_sec1_bytes(&sec1_bytes).unwrap();

        prop_assert_eq!(
            sec1_bytes.clone(),
            restored.to_sec1_bytes(),
            "SEC1 encoding should round-trip correctly"
        );
        prop_assert_eq!(
            sec1_bytes.len(),
            65,
            "Uncompressed SEC1 should be 65 bytes"
        );
    }

    // CRYPTO-INV-014: Different keys produce different shared secrets
    #[test]
    fn prop_ecdh_uniqueness(
        seed1 in seed_strategy(),
        seed2 in seed_strategy(),
        seed3 in seed_strategy()
    ) {
        prop_assume!(seed1 != seed2 && seed2 != seed3);

        let keypair_a = KasEcKeypair::from_secret_bytes(&seed1).unwrap();
        let keypair_b = KasEcKeypair::from_secret_bytes(&seed2).unwrap();
        let keypair_c = KasEcKeypair::from_secret_bytes(&seed3).unwrap();

        let shared_ab = keypair_a.diffie_hellman(&keypair_b.public_key());
        let shared_ac = keypair_a.diffie_hellman(&keypair_c.public_key());

        prop_assert_ne!(
            shared_ab, shared_ac,
            "Different peer keys should produce different shared secrets"
        );
    }
}

// Edge case tests
proptest! {
    // CRYPTO-EDGE-001: Empty message signing
    #[test]
    fn prop_empty_message_signing(seed in seed_strategy()) {
        let keypair = AgentKeypair::from_bytes(&seed).unwrap();
        let public_key = keypair.public_key();

        let empty: &[u8] = &[];
        let signature = keypair.sign(empty);

        prop_assert_eq!(signature.len(), 64, "Signature should be 64 bytes");
        prop_assert!(
            public_key.verify(empty, &signature).is_ok(),
            "Empty message signature should verify"
        );
    }

    // CRYPTO-EDGE-002: Large message signing
    #[test]
    fn prop_large_message_signing(seed in seed_strategy(), large_msg in prop::collection::vec(any::<u8>(), 10000..10001)) {
        let keypair = AgentKeypair::from_bytes(&seed).unwrap();
        let public_key = keypair.public_key();

        let signature = keypair.sign(&large_msg);

        prop_assert_eq!(signature.len(), 64, "Signature should be 64 bytes regardless of message size");
        prop_assert!(
            public_key.verify(&large_msg, &signature).is_ok(),
            "Large message signature should verify"
        );
    }

    // CRYPTO-EDGE-003: Invalid signature length detection
    #[test]
    fn prop_invalid_signature_rejection(seed in seed_strategy(), bad_sig in prop::collection::vec(any::<u8>(), 0..100)) {
        prop_assume!(bad_sig.len() != 64);

        let keypair = AgentKeypair::from_bytes(&seed).unwrap();
        let public_key = keypair.public_key();
        let message = b"test message";

        prop_assert!(
            public_key.verify(message, &bad_sig).is_err(),
            "Invalid signature length should be rejected"
        );
    }
}
