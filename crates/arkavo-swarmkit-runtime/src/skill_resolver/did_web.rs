//! Production-grade did:web PublicKeyResolver.
//!
//! Fetches https://<host>/.well-known/did.json via `ureq` (synchronous,
//! no internal tokio runtime — safe to call from sync code that itself
//! runs inside a tokio context, like the gateway's auto-launch path).
//!
//! Extracts the first Ed25519 verification method, caches the result
//! in-process. Tests cover the parsing logic against fixture JSON;
//! live HTTP fetch is exercised only by integration suite when
//! ARKAVO_DID_WEB_LIVE=1.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;

use base64::Engine;
use ed25519_dalek::VerifyingKey;
use parking_lot::RwLock;

use super::{PublicKeyResolver, ResolveError};

#[derive(Default)]
pub struct DidWebPublicKeyResolver {
    cache: Arc<RwLock<HashMap<String, VerifyingKey>>>,
    agent: OnceLock<ureq::Agent>,
}

impl DidWebPublicKeyResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Manually evict a DID's cached pubkey. Use when a signer rotates keys.
    pub fn evict(&self, did: &str) {
        self.cache.write().remove(did);
    }

    /// Clear all cached pubkeys.
    pub fn clear(&self) {
        self.cache.write().clear();
    }

    fn agent(&self) -> &ureq::Agent {
        self.agent.get_or_init(ureq::Agent::new)
    }
}

impl PublicKeyResolver for DidWebPublicKeyResolver {
    fn resolve(&self, did: &str) -> Result<VerifyingKey, ResolveError> {
        if let Some(k) = self.cache.read().get(did).copied() {
            return Ok(k);
        }
        let host =
            did.strip_prefix("did:web:")
                .ok_or_else(|| ResolveError::SignerUnresolvable {
                    id: String::new(),
                    version: String::new(),
                    did: did.to_string(),
                    reason: "only did:web supported in Phase 2".into(),
                })?;
        let url = format!("https://{host}/.well-known/did.json");
        let json: serde_json::Value = self
            .agent()
            .get(&url)
            .call()
            .map_err(|e| ResolveError::SignerUnresolvable {
                id: String::new(),
                version: String::new(),
                did: did.to_string(),
                reason: format!("did:web fetch failed: {e}"),
            })?
            .into_json()
            .map_err(|e| ResolveError::SignerUnresolvable {
                id: String::new(),
                version: String::new(),
                did: did.to_string(),
                reason: format!("did:web JSON parse failed: {e}"),
            })?;
        let key = parse_first_ed25519_pubkey(&json).map_err(|reason| {
            ResolveError::SignerUnresolvable {
                id: String::new(),
                version: String::new(),
                did: did.to_string(),
                reason,
            }
        })?;
        self.cache.write().insert(did.to_string(), key);
        Ok(key)
    }
}

fn parse_first_ed25519_pubkey(doc: &serde_json::Value) -> Result<VerifyingKey, String> {
    let methods = doc
        .get("verificationMethod")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "no verificationMethod array".to_string())?;

    let mut ed25519_seen = 0usize;
    let mut last_skip_reason: Option<String> = None;

    for vm in methods {
        let kind = vm.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind != "Ed25519VerificationKey2020" && kind != "Ed25519VerificationKey2018" {
            continue;
        }
        ed25519_seen += 1;

        // If a recognized encoding is present, return on success or surface
        // the decode error (a present-but-broken encoding is a fault, not a
        // skip — the operator wants to know).
        if let Some(b64) = vm.get("publicKeyBase64").and_then(|s| s.as_str()) {
            return decode_pubkey(b64, "publicKeyBase64");
        }
        if let Some(jwk_x) = vm
            .get("publicKeyJwk")
            .and_then(|j| j.get("x"))
            .and_then(|s| s.as_str())
        {
            return decode_pubkey(jwk_x, "publicKeyJwk.x");
        }

        // This Ed25519 method has neither recognized encoding. Don't fail
        // the whole document — record the reason and continue, so a later
        // verificationMethod entry with a recognized encoding can still
        // satisfy resolution. Reviewer M2.
        last_skip_reason = Some(format!(
            "Ed25519 verificationMethod (type={kind}) has no recognized key encoding (expected publicKeyBase64 or publicKeyJwk.x)"
        ));
    }

    if ed25519_seen == 0 {
        Err("no Ed25519 verificationMethod found".into())
    } else {
        // We saw Ed25519 method(s), but none had a recognized encoding.
        // Surface the skip reason so the operator knows the document had
        // an Ed25519 method we couldn't parse, rather than the more
        // misleading "no Ed25519 found".
        Err(last_skip_reason.unwrap_or_else(|| {
            format!("found {ed25519_seen} Ed25519 verificationMethod(s) but none had a recognized key encoding")
        }))
    }
}

fn decode_pubkey(s: &str, field: &str) -> Result<VerifyingKey, String> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(s))
        .map_err(|e| format!("{field} base64 decode: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("{field} must be 32 bytes, got {}", bytes.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    VerifyingKey::from_bytes(&arr).map_err(|e| format!("{field} key parse: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ed25519_2020_method() {
        let key_bytes = [9u8; 32];
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key_bytes);
        let doc = serde_json::json!({
            "verificationMethod": [{
                "type": "Ed25519VerificationKey2020",
                "publicKeyBase64": b64,
            }]
        });
        let key = parse_first_ed25519_pubkey(&doc).unwrap();
        assert_eq!(key.to_bytes(), key_bytes);
    }

    #[test]
    fn skips_non_ed25519_methods() {
        let doc = serde_json::json!({
            "verificationMethod": [
                {"type": "RsaVerificationKey2018", "publicKeyBase64": "x"},
            ]
        });
        let err = parse_first_ed25519_pubkey(&doc).unwrap_err();
        assert!(err.contains("no Ed25519"));
    }

    #[test]
    fn ed25519_method_without_key_encoding_errors_explicitly() {
        let doc = serde_json::json!({
            "verificationMethod": [
                {"type": "Ed25519VerificationKey2020"},
            ]
        });
        let err = parse_first_ed25519_pubkey(&doc).unwrap_err();
        assert!(err.contains("no recognized key encoding"), "got: {err}");
    }

    #[test]
    fn ed25519_with_no_encoding_skipped_for_later_method_with_encoding() {
        // Reviewer M2: a malformed Ed25519 method earlier in the array
        // must NOT prevent resolution from a working Ed25519 method later.
        let key_bytes = [11u8; 32];
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(key_bytes);
        let doc = serde_json::json!({
            "verificationMethod": [
                {"type": "Ed25519VerificationKey2020"},
                {"type": "Ed25519VerificationKey2020", "publicKeyBase64": b64},
            ]
        });
        let key = parse_first_ed25519_pubkey(&doc).unwrap();
        assert_eq!(key.to_bytes(), key_bytes);
    }

    #[test]
    fn ed25519_present_with_broken_encoding_surfaces_error() {
        // Reviewer M2: if every Ed25519 method has either a broken encoding
        // or no encoding, the error message must say so — not the misleading
        // "no Ed25519 verificationMethod found".
        let doc = serde_json::json!({
            "verificationMethod": [
                {"type": "Ed25519VerificationKey2020"},
                {"type": "Ed25519VerificationKey2018"},
            ]
        });
        let err = parse_first_ed25519_pubkey(&doc).unwrap_err();
        assert!(err.contains("no recognized key encoding"), "got: {err}");
        assert!(
            !err.contains("no Ed25519 verificationMethod found"),
            "must not surface the no-Ed25519 message when Ed25519 methods are present, got: {err}"
        );
    }

    #[test]
    fn evict_removes_cached_key() {
        let resolver = DidWebPublicKeyResolver::new();
        let key = VerifyingKey::from_bytes(&[1u8; 32]).unwrap();
        resolver
            .cache
            .write()
            .insert("did:web:example.com".into(), key);
        resolver.evict("did:web:example.com");
        assert!(resolver.cache.read().is_empty());
    }
}
