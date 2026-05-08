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

use arkavo_swarmkit::{Skill, SkillContent, SkillSource, canonical_json};
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
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
pub fn resolve_skill(skill: &Skill, cfg: &ResolverConfig) -> Result<ResolvedSkill, ResolveError> {
    let content = match skill.source {
        SkillSource::Inline => parse_inline_payload(skill)?,
        SkillSource::Registry => load_registry_payload(skill, cfg)?,
        SkillSource::TdfRef => {
            return Err(ResolveError::TdfRefNotImplemented {
                id: skill.id.clone(),
                version: skill.version.clone(),
            });
        }
    };

    let verified = verify_signature(skill, &content, cfg)?;

    Ok(ResolvedSkill {
        skill: skill.clone(),
        content,
        verified,
    })
}

fn parse_inline_payload(skill: &Skill) -> Result<SkillContent, ResolveError> {
    let payload = skill
        .payload
        .as_ref()
        .ok_or_else(|| ResolveError::InlineMissingPayload {
            id: skill.id.clone(),
            version: skill.version.clone(),
        })?;
    serde_json::from_value::<SkillContent>(payload.clone()).map_err(|e| {
        ResolveError::PayloadShape {
            id: skill.id.clone(),
            version: skill.version.clone(),
            reason: e.to_string(),
        }
    })
}

fn load_registry_payload(
    skill: &Skill,
    cfg: &ResolverConfig,
) -> Result<SkillContent, ResolveError> {
    // Skill.id is `blake3:<hex>`; strip the prefix to get the cache filename stem.
    let stem = skill.id.strip_prefix("blake3:").unwrap_or(&skill.id);
    let content_path = cfg.registry_cache.join(format!("{stem}.skill.json"));

    if !content_path.exists() {
        return Err(ResolveError::RegistryMiss {
            id: skill.id.clone(),
            version: skill.version.clone(),
            cache_path: content_path,
        });
    }

    let bytes = std::fs::read(&content_path).map_err(|e| ResolveError::Crypto {
        id: skill.id.clone(),
        version: skill.version.clone(),
        reason: format!("read {content_path:?}: {e}"),
    })?;
    serde_json::from_slice(&bytes).map_err(|e| ResolveError::PayloadShape {
        id: skill.id.clone(),
        version: skill.version.clone(),
        reason: format!("registry content parse: {e}"),
    })
}

fn verify_signature(
    skill: &Skill,
    content: &SkillContent,
    cfg: &ResolverConfig,
) -> Result<bool, ResolveError> {
    let (sig_b64, signer_did) = match (&skill.signature, &skill.signed_by) {
        (Some(s), Some(d)) => (s, d),
        _ => match cfg.verify {
            VerifyMode::Required => {
                return Err(ResolveError::SignatureMissing {
                    id: skill.id.clone(),
                    version: skill.version.clone(),
                });
            }
            VerifyMode::Optional => return Ok(false),
        },
    };

    let pubkey = cfg
        .public_key_resolver
        .resolve(signer_did)
        .map_err(|e| match e {
            ResolveError::SignerUnresolvable { did, reason, .. } => {
                ResolveError::SignerUnresolvable {
                    id: skill.id.clone(),
                    version: skill.version.clone(),
                    did,
                    reason,
                }
            }
            other => other,
        })?;

    let sig_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(sig_b64)
        .map_err(|e| ResolveError::Crypto {
            id: skill.id.clone(),
            version: skill.version.clone(),
            reason: format!("signature base64 decode failed: {e}"),
        })?;
    if sig_bytes.len() != 64 {
        return Err(ResolveError::Crypto {
            id: skill.id.clone(),
            version: skill.version.clone(),
            reason: format!("signature must be 64 bytes, got {}", sig_bytes.len()),
        });
    }
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&sig_bytes);
    let signature = Signature::from_bytes(&sig_arr);

    let canonical = canonical_skill_bytes(content).map_err(|e| ResolveError::Crypto {
        id: skill.id.clone(),
        version: skill.version.clone(),
        reason: format!("canonicalization failed: {e}"),
    })?;
    let digest = blake3::hash(&canonical);

    pubkey
        .verify(digest.as_bytes(), &signature)
        .map(|_| true)
        .map_err(|_| ResolveError::SignatureInvalid {
            id: skill.id.clone(),
            version: skill.version.clone(),
            signer_did: signer_did.clone(),
        })
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

    fn skill_with_inline_payload(content: &SkillContent, signed: Option<SignedSkill>) -> Skill {
        Skill {
            id: "skill:asset-analysis".into(),
            version: "0.1.0".into(),
            source: SkillSource::Inline,
            payload: Some(serde_json::to_value(content).unwrap()),
            signature: signed.as_ref().map(|s| s.signature_b64url.clone()),
            signed_by: signed.as_ref().map(|s| s.signed_by.clone()),
        }
    }

    fn cfg_with_mock(verify: VerifyMode, mock: MockPublicKeyResolver) -> ResolverConfig {
        ResolverConfig {
            registry_cache: std::env::temp_dir(),
            verify,
            public_key_resolver: Arc::new(mock),
        }
    }

    #[test]
    fn t1_inline_skill_round_trip() {
        let (key, did) = deterministic_test_signer();
        let content = sample_content();
        let signed = sign_skill_content(&content, did, &key);
        let skill = skill_with_inline_payload(&content, Some(signed));
        let mock = MockPublicKeyResolver::new().with_key(did, key.verifying_key());
        let resolved = resolve_skill(&skill, &cfg_with_mock(VerifyMode::Required, mock)).unwrap();
        assert_eq!(resolved.content, content);
        assert!(resolved.verified);
    }

    #[test]
    fn t2_tampered_payload_fails_verify() {
        let (key, did) = deterministic_test_signer();
        let content = sample_content();
        let signed = sign_skill_content(&content, did, &key);
        let mut tampered = content.clone();
        tampered.instructions = "Different instructions.".into();
        let skill = skill_with_inline_payload(&tampered, Some(signed));
        let mock = MockPublicKeyResolver::new().with_key(did, key.verifying_key());
        let err = resolve_skill(&skill, &cfg_with_mock(VerifyMode::Required, mock)).unwrap_err();
        assert!(matches!(err, ResolveError::SignatureInvalid { .. }));
    }

    #[test]
    fn t3_wrong_signer_did_fails_verify() {
        let (key_a, _) = deterministic_test_signer();
        let key_b = SigningKey::from_bytes(&[7u8; 32]);
        let content = sample_content();
        let signed = sign_skill_content(&content, "did:web:claimed.example", &key_a);
        let skill = skill_with_inline_payload(&content, Some(signed));
        let mock =
            MockPublicKeyResolver::new().with_key("did:web:claimed.example", key_b.verifying_key());
        let err = resolve_skill(&skill, &cfg_with_mock(VerifyMode::Required, mock)).unwrap_err();
        assert!(matches!(err, ResolveError::SignatureInvalid { .. }));
    }

    #[test]
    fn t4_missing_signature_required_mode() {
        let content = sample_content();
        let skill = skill_with_inline_payload(&content, None);
        let mock = MockPublicKeyResolver::new();
        let err = resolve_skill(&skill, &cfg_with_mock(VerifyMode::Required, mock)).unwrap_err();
        assert!(matches!(err, ResolveError::SignatureMissing { .. }));
    }

    #[test]
    fn t5_missing_signature_optional_mode() {
        let content = sample_content();
        let skill = skill_with_inline_payload(&content, None);
        let mock = MockPublicKeyResolver::new();
        let resolved = resolve_skill(&skill, &cfg_with_mock(VerifyMode::Optional, mock)).unwrap();
        assert_eq!(resolved.content, content);
        assert!(!resolved.verified);
    }

    #[test]
    fn t6_inline_missing_payload() {
        let skill = Skill {
            id: "x".into(),
            version: "0.1.0".into(),
            source: SkillSource::Inline,
            payload: None,
            signature: None,
            signed_by: None,
        };
        let mock = MockPublicKeyResolver::new();
        let err = resolve_skill(&skill, &cfg_with_mock(VerifyMode::Optional, mock)).unwrap_err();
        assert!(matches!(err, ResolveError::InlineMissingPayload { .. }));
    }

    #[test]
    fn t7_payload_shape_mismatch() {
        let skill = Skill {
            id: "x".into(),
            version: "0.1.0".into(),
            source: SkillSource::Inline,
            payload: Some(serde_json::json!({"not_a_skill_content": true})),
            signature: None,
            signed_by: None,
        };
        let mock = MockPublicKeyResolver::new();
        let err = resolve_skill(&skill, &cfg_with_mock(VerifyMode::Optional, mock)).unwrap_err();
        assert!(matches!(err, ResolveError::PayloadShape { .. }));
    }

    #[test]
    fn t8_registry_cache_hit() {
        let (key, did) = deterministic_test_signer();
        let content = sample_content();
        let signed = sign_skill_content(&content, did, &key);

        // Compute the content-addressed id the same way the resolver does.
        let canonical = canonical_skill_bytes(&content).unwrap();
        let blake3_id = blake3::hash(&canonical).to_hex().to_string();

        // Write fixture files to a temp dir.
        let dir = tempfile::tempdir().unwrap();
        let content_path = dir.path().join(format!("{blake3_id}.skill.json"));
        let sig_path = dir.path().join(format!("{blake3_id}.sig.json"));
        std::fs::write(&content_path, &canonical).unwrap();
        std::fs::write(&sig_path, serde_json::to_string(&signed).unwrap()).unwrap();

        let skill = Skill {
            id: format!("blake3:{blake3_id}"),
            version: "0.1.0".into(),
            source: SkillSource::Registry,
            payload: None,
            signature: Some(signed.signature_b64url.clone()),
            signed_by: Some(signed.signed_by.clone()),
        };
        let cfg = ResolverConfig {
            registry_cache: dir.path().to_path_buf(),
            verify: VerifyMode::Required,
            public_key_resolver: Arc::new(
                MockPublicKeyResolver::new().with_key(did, key.verifying_key()),
            ),
        };
        let resolved = resolve_skill(&skill, &cfg).unwrap();
        assert_eq!(resolved.content, content);
        assert!(resolved.verified);
    }

    #[test]
    fn t9_registry_cache_miss() {
        let dir = tempfile::tempdir().unwrap();
        let skill = Skill {
            id: "blake3:nonexistent".into(),
            version: "0.1.0".into(),
            source: SkillSource::Registry,
            payload: None,
            signature: None,
            signed_by: None,
        };
        let cfg = ResolverConfig {
            registry_cache: dir.path().to_path_buf(),
            verify: VerifyMode::Optional,
            public_key_resolver: Arc::new(MockPublicKeyResolver::new()),
        };
        let err = resolve_skill(&skill, &cfg).unwrap_err();
        assert!(matches!(err, ResolveError::RegistryMiss { .. }));
    }
}
