//! Signed JSON envelopes for authority evidence and receipts.
//!
//! The signature covers the payload bytes exactly as sent (base64url-wrapped), so nothing is
//! re-serialized before verification. A type string and the key ID are bound into the signed
//! bytes, so an envelope of one type (a receipt) can never be presented as another (a grant).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};

const DOMAIN: &[u8] = b"arkavo-tap-envelope-v1";

/// A payload, its type, the signing key's ID and an Ed25519 signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SignedEnvelope {
    pub typ: String,
    pub kid: String,
    pub alg: String,
    /// The payload bytes, base64url without padding.
    pub payload: String,
    pub signature: String,
}

fn signed_bytes(typ: &str, kid: &str, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(DOMAIN.len() + typ.len() + kid.len() + payload.len() + 3);
    out.extend_from_slice(DOMAIN);
    out.push(0);
    out.extend_from_slice(typ.as_bytes());
    out.push(0);
    out.extend_from_slice(kid.as_bytes());
    out.push(0);
    out.extend_from_slice(payload);
    out
}

/// Signs `payload` as an envelope of type `typ`.
pub fn seal(typ: &str, kid: &str, key: &SigningKey, payload: &[u8]) -> SignedEnvelope {
    let signature = key.sign(&signed_bytes(typ, kid, payload));
    SignedEnvelope {
        typ: typ.to_owned(),
        kid: kid.to_owned(),
        alg: "ed25519".to_owned(),
        payload: URL_SAFE_NO_PAD.encode(payload),
        signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    }
}

/// The payload of an envelope of type `typ`, if `key` signed it.
pub fn open(envelope: &SignedEnvelope, typ: &str, key: &VerifyingKey) -> Result<Vec<u8>, String> {
    if envelope.typ != typ {
        return Err(format!("expected a {typ}, got a {}", envelope.typ));
    }
    if envelope.alg != "ed25519" {
        return Err(format!(
            "envelope algorithm {} is not accepted",
            envelope.alg
        ));
    }
    let payload = URL_SAFE_NO_PAD
        .decode(&envelope.payload)
        .map_err(|_| "the payload is not base64url".to_owned())?;
    let signature = URL_SAFE_NO_PAD
        .decode(&envelope.signature)
        .ok()
        .and_then(|s| Signature::from_slice(&s).ok())
        .ok_or("the signature is not an Ed25519 signature")?;
    key.verify_strict(&signed_bytes(typ, &envelope.kid, &payload), &signature)
        .map_err(|_| "the envelope signature does not verify".to_owned())?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_only_what_its_key_signed_as_its_type() {
        let key = SigningKey::from_bytes(&[3u8; 32]);
        let env = seal("local-test-grant", "issuer-1", &key, b"{\"a\":1}");
        let vk = key.verifying_key();
        assert_eq!(open(&env, "local-test-grant", &vk).unwrap(), b"{\"a\":1}");
        assert!(open(&env, "simulated-receipt", &vk).is_err());
        let other = SigningKey::from_bytes(&[4u8; 32]).verifying_key();
        assert!(open(&env, "local-test-grant", &other).is_err());

        let mut retyped = env.clone();
        retyped.typ = "simulated-receipt".into();
        assert!(open(&retyped, "simulated-receipt", &vk).is_err());
        let mut rekeyed = env.clone();
        rekeyed.kid = "issuer-2".into();
        assert!(open(&rekeyed, "local-test-grant", &vk).is_err());
        let mut altered = env.clone();
        altered.payload = URL_SAFE_NO_PAD.encode(b"{\"a\":2}");
        assert!(open(&altered, "local-test-grant", &vk).is_err());
        let mut bad = env.clone();
        bad.alg = "none".into();
        assert!(open(&bad, "local-test-grant", &vk).is_err());
        bad = env.clone();
        bad.payload = "***".into();
        assert!(open(&bad, "local-test-grant", &vk).is_err());
        bad = env;
        bad.signature = "AQID".into();
        assert!(open(&bad, "local-test-grant", &vk).is_err());
    }
}
