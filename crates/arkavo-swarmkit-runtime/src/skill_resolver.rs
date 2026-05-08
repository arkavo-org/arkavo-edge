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

use arkavo_swarmkit::{Skill, SkillContent};
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::Serialize;

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

/// Sign `SkillContent` with the given ed25519 `_private_key`.
///
/// Returns a `SignedSkill` carrying the base64url signature and signer DID.
/// The `SigningKey` import is used here to anchor the type; Task 3 fills
/// the implementation.
pub fn sign_skill_content(
    _content: &SkillContent,
    _signer_did: &str,
    _private_key: &SigningKey,
) -> SignedSkill {
    unimplemented!("Task 3 lands the signer")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_compile_and_mock_resolver_returns_unresolvable() {
        let mock = MockPublicKeyResolver::new();
        let err = mock.resolve("did:web:nope").unwrap_err();
        assert!(matches!(err, ResolveError::SignerUnresolvable { .. }));
    }
}
