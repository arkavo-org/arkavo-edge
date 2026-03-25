//! Detached JWS (JSON Web Signature) with Ed25519 per RFC 7515.
//!
//! Produces compact-serialized detached signatures where the payload
//! is omitted from the token: `base64url(header)..base64url(signature)`.

use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;

use crate::IdentityError;

/// A detached JWS signature (RFC 7515 Appendix F).
#[derive(Debug, Clone)]
pub struct DetachedJws {
    /// Base64url-encoded protected header
    pub protected: String,
    /// Base64url-encoded Ed25519 signature
    pub signature: String,
}

impl DetachedJws {
    /// Sign a payload with the given keypair, producing a detached JWS.
    ///
    /// Header: `{"alg":"EdDSA","kid":"<did:key>"}`
    /// Signing input: `base64url(header) || "." || base64url(payload)`
    pub fn sign(keypair: &AgentKeypair, did: &str, payload: &[u8]) -> Self {
        let header = format!(r#"{{"alg":"EdDSA","kid":"{did}"}}"#);
        let protected = URL_SAFE_NO_PAD.encode(header.as_bytes());
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload);

        let signing_input = format!("{protected}.{payload_b64}");
        let sig_bytes = keypair.sign(signing_input.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(&sig_bytes);

        Self {
            protected,
            signature,
        }
    }

    /// Verify a detached JWS against the given payload and public key.
    pub fn verify(&self, public_key: &AgentPublicKey, payload: &[u8]) -> Result<(), IdentityError> {
        let payload_b64 = URL_SAFE_NO_PAD.encode(payload);
        let signing_input = format!("{}.{payload_b64}", self.protected);

        let sig_bytes = URL_SAFE_NO_PAD
            .decode(&self.signature)
            .map_err(|e| IdentityError::Jws(format!("base64 decode: {e}")))?;

        public_key
            .verify(signing_input.as_bytes(), &sig_bytes)
            .map_err(|e| IdentityError::Jws(format!("signature verification: {e}")))
    }

    /// Compact serialization: `header..signature` (empty middle = detached payload).
    pub fn to_compact(&self) -> String {
        format!("{}..{}", self.protected, self.signature)
    }

    /// Parse from compact serialization.
    pub fn from_compact(s: &str) -> Result<Self, IdentityError> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 || !parts[1].is_empty() {
            return Err(IdentityError::Jws(
                "expected detached JWS format: header..signature".to_string(),
            ));
        }
        Ok(Self {
            protected: parts[0].to_string(),
            signature: parts[2].to_string(),
        })
    }
}

/// Produce deterministic JSON bytes with sorted object keys.
///
/// Sufficient for MCP-I L1 proof hashing. Recursively sorts all
/// object keys and uses compact (no whitespace) serialization.
pub fn canonical_json(value: &Value) -> Vec<u8> {
    let sorted = sort_value(value);
    serde_json::to_vec(&sorted).unwrap_or_default()
}

fn sort_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted: Vec<(&String, Value)> =
                map.iter().map(|(k, v)| (k, sort_value(v))).collect();
            sorted.sort_by(|a, b| a.0.cmp(b.0));
            let ordered: serde_json::Map<String, Value> =
                sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect();
            Value::Object(ordered)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sort_value).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_round_trip() {
        let keypair = AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();
        let payload = b"hello world";

        let jws = DetachedJws::sign(&keypair, &did, payload);
        assert!(jws.verify(&keypair.public_key(), payload).is_ok());
    }

    #[test]
    fn verify_rejects_tampered_payload() {
        let keypair = AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();

        let jws = DetachedJws::sign(&keypair, &did, b"original");
        assert!(jws.verify(&keypair.public_key(), b"tampered").is_err());
    }

    #[test]
    fn verify_rejects_wrong_key() {
        let keypair = AgentKeypair::generate();
        let other = AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();
        let payload = b"data";

        let jws = DetachedJws::sign(&keypair, &did, payload);
        assert!(jws.verify(&other.public_key(), payload).is_err());
    }

    #[test]
    fn compact_round_trip() {
        let keypair = AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();

        let jws = DetachedJws::sign(&keypair, &did, b"payload");
        let compact = jws.to_compact();

        // Detached format: header..signature (two dots, empty middle)
        let parts: Vec<&str> = compact.split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts[1].is_empty());

        let parsed = DetachedJws::from_compact(&compact).unwrap();
        assert_eq!(parsed.protected, jws.protected);
        assert_eq!(parsed.signature, jws.signature);

        // Parsed JWS still verifies
        assert!(parsed.verify(&keypair.public_key(), b"payload").is_ok());
    }

    #[test]
    fn from_compact_rejects_invalid_format() {
        assert!(DetachedJws::from_compact("not-a-jws").is_err());
        assert!(DetachedJws::from_compact("a.b.c").is_err()); // non-empty middle
    }

    #[test]
    fn canonical_json_sorts_keys() {
        let value = serde_json::json!({"z": 1, "a": 2, "m": {"b": 3, "a": 4}});
        let bytes = canonical_json(&value);
        let s = String::from_utf8(bytes).unwrap();
        assert_eq!(s, r#"{"a":2,"m":{"a":4,"b":3},"z":1}"#);
    }

    #[test]
    fn canonical_json_handles_arrays() {
        let value = serde_json::json!({"items": [{"z": 1, "a": 2}, "text"]});
        let bytes = canonical_json(&value);
        let s = String::from_utf8(bytes).unwrap();
        assert_eq!(s, r#"{"items":[{"a":2,"z":1},"text"]}"#);
    }

    #[test]
    fn canonical_json_deterministic() {
        let value = serde_json::json!({"b": true, "a": null, "c": [1, 2]});
        let b1 = canonical_json(&value);
        let b2 = canonical_json(&value);
        assert_eq!(b1, b2);
    }
}
