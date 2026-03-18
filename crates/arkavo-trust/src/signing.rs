//! Trust score and event signing/verification using Ed25519 + JCS

use arkavo_crypto::{AgentKeypair, AgentPublicKey, CryptoError};
use base64::{engine::general_purpose, Engine as _};

use crate::jcs::canonicalize;
use crate::types::{TrustEvent, TrustScore, TrustSignature};

/// Sign a TrustScore, setting the signature field.
///
/// The signature is computed over the JCS-canonicalized JSON of the score
/// with the `signature` field removed (per MCP-T Section 4.2.5).
pub fn sign_trust_score(score: &mut TrustScore, keypair: &AgentKeypair) {
    score.signature = None;
    let canonical = score_to_canonical_json(score);
    let sig_bytes = keypair.sign(canonical.as_bytes());
    let public_key = keypair.public_key();

    score.signature = Some(TrustSignature {
        algorithm: "Ed25519".to_string(),
        public_key: did_key_short(&public_key),
        value: general_purpose::URL_SAFE_NO_PAD.encode(&sig_bytes),
    });
}

/// Verify the signature on a TrustScore.
pub fn verify_trust_score(score: &TrustScore) -> Result<(), SigningError> {
    let sig = score
        .signature
        .as_ref()
        .ok_or(SigningError::MissingSignature)?;

    let public_key = AgentPublicKey::from_did_key(&format!("did:key:{}", sig.public_key))
        .map_err(|e| SigningError::InvalidKey(e.to_string()))?;

    let sig_bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(&sig.value)
        .map_err(|e| SigningError::InvalidSignature(e.to_string()))?;

    // Reconstruct canonical JSON without signature field
    let mut score_copy = score.clone();
    score_copy.signature = None;
    let canonical = score_to_canonical_json(&score_copy);

    public_key
        .verify(canonical.as_bytes(), &sig_bytes)
        .map_err(|e| SigningError::VerificationFailed(e.to_string()))
}

/// Sign a TrustEvent, setting the signature field.
///
/// The signature is computed over the JCS-canonicalized JSON of the event
/// with `signature` and `co_signatures` removed (per MCP-T Section 6.4).
pub fn sign_trust_event(event: &mut TrustEvent, keypair: &AgentKeypair) {
    event.signature = None;
    let saved_cosigs = event.co_signatures.take();
    let canonical = event_to_canonical_json(event);
    event.co_signatures = saved_cosigs;

    let sig_bytes = keypair.sign(canonical.as_bytes());
    let public_key = keypair.public_key();

    event.signature = Some(TrustSignature {
        algorithm: "Ed25519".to_string(),
        public_key: did_key_short(&public_key),
        value: general_purpose::URL_SAFE_NO_PAD.encode(&sig_bytes),
    });
}

/// Verify the signature on a TrustEvent.
pub fn verify_trust_event(event: &TrustEvent) -> Result<(), SigningError> {
    let sig = event
        .signature
        .as_ref()
        .ok_or(SigningError::MissingSignature)?;

    let public_key = AgentPublicKey::from_did_key(&format!("did:key:{}", sig.public_key))
        .map_err(|e| SigningError::InvalidKey(e.to_string()))?;

    let sig_bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(&sig.value)
        .map_err(|e| SigningError::InvalidSignature(e.to_string()))?;

    // Reconstruct canonical JSON without signature and co_signatures
    let mut event_copy = event.clone();
    event_copy.signature = None;
    event_copy.co_signatures = None;
    let canonical = event_to_canonical_json(&event_copy);

    public_key
        .verify(canonical.as_bytes(), &sig_bytes)
        .map_err(|e| SigningError::VerificationFailed(e.to_string()))
}

fn score_to_canonical_json(score: &TrustScore) -> String {
    let value = serde_json::to_value(score).expect("TrustScore is always serializable");
    canonicalize(&value)
}

fn event_to_canonical_json(event: &TrustEvent) -> String {
    let value = serde_json::to_value(event).expect("TrustEvent is always serializable");
    canonicalize(&value)
}

/// Extract the short form of a DID:key (the `z6Mk...` part).
fn did_key_short(public_key: &AgentPublicKey) -> String {
    let did = public_key.to_did_key();
    did.strip_prefix("did:key:").unwrap_or(&did).to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum SigningError {
    #[error("Missing signature")]
    MissingSignature,
    #[error("Invalid key: {0}")]
    InvalidKey(String),
    #[error("Invalid signature encoding: {0}")]
    InvalidSignature(String),
    #[error("Signature verification failed: {0}")]
    VerificationFailed(String),
    #[error("Crypto error: {0}")]
    Crypto(#[from] CryptoError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn test_score() -> TrustScore {
        let mut dims = HashMap::new();
        dims.insert(
            "performance".to_string(),
            DimensionScore {
                value: 780,
                confidence: 0.72,
                evidence_count: 156,
            },
        );
        dims.insert(
            "consistency".to_string(),
            DimensionScore {
                value: 800,
                confidence: 0.83,
                evidence_count: 892,
            },
        );

        TrustScore {
            schema_version: SCHEMA_VERSION.to_string(),
            subject_id: "did:key:z6MkSubject".to_string(),
            provider_id: "did:key:z6MkProvider".to_string(),
            score: TrustScoreValue {
                composite: 742,
                dimensions: dims,
            },
            domain: Some("code-execution".to_string()),
            domain_match: Some(true),
            validity: ValidityWindow {
                issued_at: Utc::now(),
                expires_at: Utc::now() + chrono::Duration::hours(1),
                max_age_seconds: 3600,
            },
            metadata: None,
            authorized: Some(true),
            signature: None,
        }
    }

    fn test_event() -> TrustEvent {
        TrustEvent {
            event_id: "evt_01TEST".to_string(),
            event_type: "contract.completed".to_string(),
            subject_id: "did:key:z6MkSubject".to_string(),
            issuer_id: "did:key:z6MkIssuer".to_string(),
            timestamp: Utc::now(),
            payload: serde_json::json!({"contract_id": "ctr_1", "outcome": "success"}),
            dimensions_affected: Some(vec!["performance".to_string()]),
            signature: None,
            co_signatures: None,
        }
    }

    #[test]
    fn sign_and_verify_trust_score() {
        let keypair = AgentKeypair::generate();
        let mut score = test_score();

        sign_trust_score(&mut score, &keypair);
        assert!(score.signature.is_some());

        let sig = score.signature.as_ref().unwrap();
        assert_eq!(sig.algorithm, "Ed25519");
        assert!(!sig.value.is_empty());

        // Verification should succeed
        verify_trust_score(&score).unwrap();
    }

    #[test]
    fn tampered_score_fails_verification() {
        let keypair = AgentKeypair::generate();
        let mut score = test_score();

        sign_trust_score(&mut score, &keypair);

        // Tamper with the score
        score.score.composite = 999;

        let result = verify_trust_score(&score);
        assert!(result.is_err());
    }

    #[test]
    fn sign_and_verify_trust_event() {
        let keypair = AgentKeypair::generate();
        let mut event = test_event();

        sign_trust_event(&mut event, &keypair);
        assert!(event.signature.is_some());

        verify_trust_event(&event).unwrap();
    }

    #[test]
    fn tampered_event_fails_verification() {
        let keypair = AgentKeypair::generate();
        let mut event = test_event();

        sign_trust_event(&mut event, &keypair);

        // Tamper
        event.event_type = "contract.failed".to_string();

        let result = verify_trust_event(&event);
        assert!(result.is_err());
    }

    #[test]
    fn missing_signature_returns_error() {
        let score = test_score();
        let result = verify_trust_score(&score);
        assert!(matches!(result, Err(SigningError::MissingSignature)));
    }

    #[test]
    fn wrong_key_fails_verification() {
        let keypair1 = AgentKeypair::generate();
        let keypair2 = AgentKeypair::generate();

        let mut score = test_score();
        sign_trust_score(&mut score, &keypair1);

        // Replace public key with different key
        if let Some(ref mut sig) = score.signature {
            sig.public_key = did_key_short(&keypair2.public_key());
        }

        let result = verify_trust_score(&score);
        assert!(result.is_err());
    }

    #[test]
    fn cosignatures_excluded_from_event_signing() {
        let keypair = AgentKeypair::generate();
        let mut event = test_event();

        // Add co-signatures before signing
        event.co_signatures = Some(vec![CoSignature {
            signer_id: "did:key:z6MkCosigner".to_string(),
            algorithm: "Ed25519".to_string(),
            public_key: "z6MkCosigner".to_string(),
            value: "fake_sig".to_string(),
        }]);

        sign_trust_event(&mut event, &keypair);

        // Co-signatures should be preserved
        assert!(event.co_signatures.is_some());
        assert_eq!(event.co_signatures.as_ref().unwrap().len(), 1);

        // Verification should still work (co_signatures excluded from hash)
        verify_trust_event(&event).unwrap();
    }

    #[test]
    fn deterministic_signatures() {
        let keypair = AgentKeypair::generate();
        let mut score1 = test_score();
        let mut score2 = score1.clone();

        sign_trust_score(&mut score1, &keypair);
        sign_trust_score(&mut score2, &keypair);

        // Ed25519 is deterministic — same input + same key = same signature
        assert_eq!(
            score1.signature.as_ref().unwrap().value,
            score2.signature.as_ref().unwrap().value
        );
    }
}
