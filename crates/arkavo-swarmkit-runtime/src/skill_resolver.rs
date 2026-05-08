//! Skill resolver per spec §7.3 / §11-SPEC-2.
//!
//! Resolves a `Skill` reference to a verified `ResolvedSkill` carrying
//! the parsed `SkillContent`.
//!
//! Eager resolution happens at `SwarmFlight::launch` when
//! `LaunchOptions::resolver_config` is `Some`.
//!
//! Source modes:
//!
//! * `inline` — payload is embedded in the manifest, signed and verified.
//! * `registry` — content-addressed file in a local cache, signed.
//! * `tdf-ref` — explicit roadmap variant; returns `TdfRefNotImplemented`.

use std::path::PathBuf;
use std::sync::Arc;

use arkavo_swarmkit::{Skill, SkillContent, canonical_json};
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use serde::Serialize;

/// Compute the canonical-form bytes a SkillContent gets signed over.
///
/// Uses arkavo_swarmkit::canonical_json (JCS, RFC 8785 — sorted keys, no
/// insignificant whitespace).
///
/// The same helper backs `kit.id`, so the canonicalization is consistent
/// across the SwarmKit surface.
fn canonical_skill_bytes(content: &SkillContent) -> Result<Vec<u8>, serde_json::Error> {
    let value = serde_json::to_value(content)?;
    Ok(canonical_json(&value).into_bytes())
}

#[derive(Debug, Clone)]
pub struct ResolvedSkill {
    pub skill: Skill,
    pub content: SkillContent,
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    Required,
    Optional,
}

pub trait PublicKeyResolver: Send + Sync {
    fn resolve(&self, did: &str) -> Result<VerifyingKey, ResolveError>;
}

pub struct ResolverConfig {
    pub registry_cache: PathBuf,
    pub verify: VerifyMode,
    pub public_key_resolver: Arc<dyn PublicKeyResolver>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignedSkill {
    pub signature_b64url: String,
    pub signed_by: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    #[error("skill {id}@{version}: source `inline` requires `payload`")]
    InlineMissingPayload { id: String, version: String },
    #[error("skill {id}@{version}: payload does not parse as SkillContent: {reason}")]
    PayloadShape {
        id: String,
        version: String,
        reason: String,
    },
    #[error("skill {id}@{version}: not found in registry cache at {cache_path:?}")]
    RegistryMiss {
        id: String,
        version: String,
        cache_path: PathBuf,
    },
    #[error("skill {id}@{version}: tdf-ref source not implemented (roadmap)")]
    TdfRefNotImplemented { id: String, version: String },
    #[error("skill {id}@{version}: signature missing under VerifyMode::Required")]
    SignatureMissing { id: String, version: String },
    #[error("skill {id}@{version}: signature did not verify against signer {signer_did}")]
    SignatureInvalid {
        id: String,
        version: String,
        signer_did: String,
    },
    #[error("skill {id}@{version}: signed_by DID {did} could not be resolved: {reason}")]
    SignerUnresolvable {
        id: String,
        version: String,
        did: String,
        reason: String,
    },
    #[error("skill {id}@{version}: cryptographic error: {reason}")]
    Crypto {
        id: String,
        version: String,
        reason: String,
    },
}

#[cfg(test)]
pub(crate) struct MockPublicKeyResolver {
    keys: std::collections::HashMap<String, VerifyingKey>,
}

#[cfg(test)]
impl MockPublicKeyResolver {
    pub fn new() -> Self {
        Self {
            keys: std::collections::HashMap::new(),
        }
    }

    pub fn with_key(mut self, did: &str, key: VerifyingKey) -> Self {
        self.keys.insert(did.to_string(), key);
        self
    }
}

#[cfg(test)]
impl PublicKeyResolver for MockPublicKeyResolver {
    fn resolve(&self, did: &str) -> Result<VerifyingKey, ResolveError> {
        self.keys
            .get(did)
            .copied()
            .ok_or_else(|| ResolveError::SignerUnresolvable {
                id: String::new(),
                version: String::new(),
                did: did.to_string(),
                reason: "no key in mock resolver".into(),
            })
    }
}

/// Resolve a `Skill` reference to a verified `ResolvedSkill`.
pub fn resolve_skill(_skill: &Skill, _cfg: &ResolverConfig) -> Result<ResolvedSkill, ResolveError> {
    unimplemented!("Task 4 lands the inline branch")
}

/// Sign `SkillContent` with the given ed25519 `private_key`.
///
/// Returns a `SignedSkill` carrying the base64url signature and signer DID.
/// The digest is BLAKE3(canonical_json(content)); ed25519 signs the 32-byte
/// digest. Deterministic for a given (key, content) pair.
pub fn sign_skill_content(
    content: &SkillContent,
    signer_did: &str,
    private_key: &SigningKey,
) -> SignedSkill {
    let canonical =
        canonical_skill_bytes(content).expect("SkillContent serialization must not fail");
    let digest = blake3::hash(&canonical);
    let signature = private_key.sign(digest.as_bytes());
    SignedSkill {
        signature_b64url: base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(signature.to_bytes()),
        signed_by: signer_did.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deterministic_test_signer() -> (SigningKey, &'static str) {
        let key = SigningKey::from_bytes(&[42u8; 32]);
        (key, "did:web:test.arkavo.com")
    }

    fn sample_content() -> SkillContent {
        SkillContent {
            name: "asset-analysis".into(),
            description: "Summarize a source asset".into(),
            instructions: "Extract three to five selling points.".into(),
            resources: vec![],
        }
    }

    #[test]
    fn types_compile_and_mock_resolver_returns_unresolvable() {
        let mock = MockPublicKeyResolver::new();
        let err = mock.resolve("did:web:nope").unwrap_err();
        assert!(matches!(err, ResolveError::SignerUnresolvable { .. }));
    }

    #[test]
    fn sign_skill_content_is_deterministic() {
        let (key, did) = deterministic_test_signer();
        let content = sample_content();
        let s1 = sign_skill_content(&content, did, &key);
        let s2 = sign_skill_content(&content, did, &key);
        assert_eq!(s1.signature_b64url, s2.signature_b64url);
        assert_eq!(s1.signed_by, did);
    }
}
