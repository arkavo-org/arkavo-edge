//! Production-grade did:web PublicKeyResolver.
//!
//! Fetches https://<host>/.well-known/did.json, extracts the first
//! Ed25519 verification method, caches the result in-process.
//!
//! Tests cover the parsing logic against fixture JSON; live HTTP
//! fetch is exercised only by the integration suite when
//! ARKAVO_DID_WEB_LIVE=1.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use base64::Engine;
use ed25519_dalek::VerifyingKey;
use parking_lot::RwLock;

use super::{PublicKeyResolver, ResolveError};

/// Production did:web resolver. The `reqwest::blocking::Client` is built
/// lazily on first use so that constructing `ResolverConfig::default()`
/// inside an async context (e.g. from `auto_launch_from_environment`) does
/// not create — and later panic-drop — a blocking tokio runtime.
pub struct DidWebPublicKeyResolver {
    cache: Arc<RwLock<HashMap<String, VerifyingKey>>>,
    client: OnceLock<reqwest::blocking::Client>,
}

impl Default for DidWebPublicKeyResolver {
    fn default() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            client: OnceLock::new(),
        }
    }
}

impl DidWebPublicKeyResolver {
    pub fn new() -> Self {
        Self::default()
    }

    fn client(&self) -> &reqwest::blocking::Client {
        self.client.get_or_init(reqwest::blocking::Client::default)
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
            .client()
            .get(&url)
            .send()
            .and_then(|r| r.json())
            .map_err(|e| ResolveError::SignerUnresolvable {
                id: String::new(),
                version: String::new(),
                did: did.to_string(),
                reason: format!("did:web fetch failed: {e}"),
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
    for vm in methods {
        let kind = vm.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind != "Ed25519VerificationKey2020" && kind != "Ed25519VerificationKey2018" {
            continue;
        }
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
    }
    Err("no Ed25519 verificationMethod found".into())
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
}
