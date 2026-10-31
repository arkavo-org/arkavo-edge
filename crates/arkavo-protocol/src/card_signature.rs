//! Agent Card signatures as A2A v1.0 defines them (§8.4): a JWS whose
//! payload is the card's RFC 8785 canonical form, without its `signatures`.
//!
//! The signer is the agent's workload key, named by its `did:key` in the
//! `kid`, so a verifier needs neither a network fetch nor a key store: the
//! DID is the public key. Only ES256 over a P-256 `did:key` is accepted.
//!
//! The payload is the card as served. A verifier that first rebuilds the
//! card from A2A v1.0 types drops the fields this card has and v1.0 lacks,
//! so it will not match until the card itself takes the v1.0 shape (#636).

use crate::types::{AgentCard, AgentCardSignature};
use arkavo_crypto::P256VerifyingKey;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;

const ALGORITHM: &str = "ES256";

/// The largest integer every RFC 8785 implementation writes exactly: JCS
/// numbers are IEEE doubles.
const EXACT_INTEGER: u64 = 1 << 53;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CardSignatureError {
    #[error("the card is not signed")]
    Unsigned,
    #[error("the card has no canonical form: {0}")]
    Canonical(String),
    #[error("signing the card failed: {0}")]
    Signer(String),
    #[error("no signature on the card verifies: {0}")]
    Invalid(String),
}

/// Sign `card` with the P-256 key that `key_id` names, replacing any
/// signatures it carries. `sign` returns ECDSA P-256 SHA-256 as `r || s`.
pub fn sign<E: std::fmt::Display>(
    card: &mut AgentCard,
    key_id: &str,
    sign: impl FnOnce(&[u8]) -> Result<[u8; 64], E>,
) -> Result<(), CardSignatureError> {
    card.signatures.clear();
    let unsigned =
        serde_json::to_value(&*card).map_err(|e| CardSignatureError::Canonical(e.to_string()))?;
    let payload = canonical(&unsigned)?;
    let header = serde_json::json!({ "alg": ALGORITHM, "typ": "JOSE", "kid": key_id });
    let protected = URL_SAFE_NO_PAD.encode(header.to_string());
    let signature = sign(signing_input(&protected, &payload).as_bytes())
        .map_err(|e| CardSignatureError::Signer(e.to_string()))?;
    card.signatures.push(AgentCardSignature {
        protected,
        signature: URL_SAFE_NO_PAD.encode(signature),
        header: None,
    });
    Ok(())
}

/// The `did:key` whose key signed `card`, checked against the card exactly
/// as it was received.
pub fn verify(card: &Value) -> Result<String, CardSignatureError> {
    let signatures = card
        .get("signatures")
        .and_then(Value::as_array)
        .filter(|signatures| !signatures.is_empty())
        .ok_or(CardSignatureError::Unsigned)?;
    let mut unsigned = card.clone();
    if let Some(fields) = unsigned.as_object_mut() {
        fields.remove("signatures");
    }
    let payload = canonical(&unsigned)?;
    let mut why = String::new();
    for signature in signatures {
        match verify_one(signature, &payload) {
            Ok(did) => return Ok(did),
            Err(reason) => why = reason,
        }
    }
    Err(CardSignatureError::Invalid(why))
}

fn verify_one(signature: &Value, payload: &str) -> Result<String, String> {
    let protected = signature
        .get("protected")
        .and_then(Value::as_str)
        .ok_or("a signature has no protected header")?;
    let header: Value = URL_SAFE_NO_PAD
        .decode(protected)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or("the protected header is not base64url JSON")?;
    if header.get("alg").and_then(Value::as_str) != Some(ALGORITHM) {
        return Err(format!("the signature is not {ALGORITHM}"));
    }
    let kid = header
        .get("kid")
        .and_then(Value::as_str)
        .ok_or("the protected header names no key")?;
    let (key, did) = P256VerifyingKey::from_did_key_id(kid)
        .map_err(|e| format!("{kid} is not a P-256 did:key key id: {e}"))?;
    let value = signature
        .get("signature")
        .and_then(Value::as_str)
        .and_then(|encoded| URL_SAFE_NO_PAD.decode(encoded).ok())
        .ok_or("the signature is not base64url")?;
    // JWS ES256 is the 64-byte r || s; DER is not a JWS signature.
    if value.len() != 64 {
        return Err("the signature is not a 64-byte ES256 value".to_string());
    }
    key.verify(signing_input(protected, payload).as_bytes(), &value)
        .map_err(|_| format!("the signature by {did} does not match the card"))?;
    Ok(did.to_string())
}

fn signing_input(protected: &str, payload: &str) -> String {
    format!("{protected}.{}", URL_SAFE_NO_PAD.encode(payload))
}

/// RFC 8785 canonical JSON for the values a card holds. Numbers that are
/// not exact integers are refused rather than written: the card has none,
/// and RFC 8785's form for them would have to match every other
/// implementation's to the digit.
fn canonical(value: &Value) -> Result<String, CardSignatureError> {
    let mut out = String::new();
    write_canonical(value, &mut out)?;
    Ok(out)
}

fn write_canonical(value: &Value, out: &mut String) -> Result<(), CardSignatureError> {
    match value {
        Value::Null | Value::Bool(_) => out.push_str(&value.to_string()),
        Value::Number(number) => {
            let exact = number
                .as_u64()
                .map(|n| n <= EXACT_INTEGER)
                .or_else(|| number.as_i64().map(|n| n.unsigned_abs() <= EXACT_INTEGER))
                .unwrap_or(false);
            if !exact {
                return Err(CardSignatureError::Canonical(format!(
                    "{number} is not an exact integer"
                )));
            }
            out.push_str(&number.to_string());
        }
        // serde_json escapes strings exactly as RFC 8785 does: the two-letter
        // forms, \u00xx in lower case for other control characters, and
        // everything else as it is.
        Value::String(_) => out.push_str(&value.to_string()),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out)?;
            }
            out.push(']');
        }
        Value::Object(fields) => {
            let mut fields: Vec<_> = fields.iter().collect();
            // RFC 8785 orders members by the UTF-16 code units of their names.
            fields.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, (name, field)) in fields.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(name.clone()).to_string());
                out.push(':');
                write_canonical(field, out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
