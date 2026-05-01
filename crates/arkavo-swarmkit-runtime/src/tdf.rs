//! TDF envelope wrap/unwrap for SwarmKit manifests (spec §6).
//!
//! Producers serialize a `Manifest` to canonical JSON, encrypt the bytes
//! through any [`TdfEncryptor`], and ship the resulting [`TdfManifest`]
//! as a `.swarmkit.tdf` payload. Orchestrators reverse the flow: decrypt
//! through a [`TdfDecryptor`], parse the canonical JSON back into a
//! `Manifest`, and run cross-block validation before launching a flight.
//!
//! This module is gated behind the `tdf` feature so that headless /
//! in-process flight orchestration can avoid the opentdf-rs dependency
//! tree. Out of scope for now: KAS-gated decryption (spec §6.3),
//! per-role TDF policy construction (spec §6.4), and `.swarmkit.tdf`
//! file-format serialization. Those land in subsequent slices.

use arkavo_swarmkit::{Manifest, ParseError, parse_json};
use arkavo_tdf::{Policy, PolicyBuilder, TdfDecryptor, TdfEncryptor, TdfError, TdfManifest};

use crate::canonical_full_manifest;

/// Errors that can occur while wrapping or unwrapping a SwarmKit manifest.
#[derive(Debug, thiserror::Error)]
pub enum TdfEnvelopeError {
    #[error("serialize manifest: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("encrypt manifest: {0}")]
    Encrypt(TdfError),
    #[error("decrypt manifest: {0}")]
    Decrypt(TdfError),
    #[error("parse decrypted manifest: {0}")]
    Parse(#[from] ParseError),
    #[error("decrypted payload is not valid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

/// Wrap a SwarmKit manifest in a TDF envelope per spec §6.
///
/// The manifest is canonicalized (sorted keys, no insignificant
/// whitespace) before encryption so the wrapped payload is deterministic
/// for a given input. The supplied [`Policy`] is the §6.3 SwarmKit-level
/// orchestrator gate — it controls who can request the wrapped key from
/// the KAS and unwrap the manifest. Use [`swarmkit_orchestrator_policy`]
/// for the spec's recommended baseline.
pub async fn wrap_manifest<E: TdfEncryptor>(
    manifest: &Manifest,
    encryptor: &E,
    policy: &Policy,
) -> Result<TdfManifest, TdfEnvelopeError> {
    let canonical = canonical_full_manifest(manifest)?;
    encryptor
        .encrypt(canonical.as_bytes(), policy)
        .await
        .map_err(TdfEnvelopeError::Encrypt)
}

/// Reverse of [`wrap_manifest`]: decrypt the TDF envelope, parse the
/// resulting JSON back into a `Manifest`, and run cross-block validation.
///
/// The returned manifest is fully validated — same guarantees as
/// [`arkavo_swarmkit::parse_json`].
pub async fn unwrap_manifest<D: TdfDecryptor>(
    tdf: &TdfManifest,
    decryptor: &D,
) -> Result<Manifest, TdfEnvelopeError> {
    let plaintext = decryptor
        .decrypt(tdf)
        .await
        .map_err(TdfEnvelopeError::Decrypt)?;
    let json = String::from_utf8(plaintext)?;
    Ok(parse_json(&json)?)
}

/// Build the spec §6.3 SwarmKit-level TDF Attribute Release Policy.
///
/// Gates orchestrator decryption on two attributes:
/// * `https://attr.arkavo.com/role/orchestrator`
/// * `https://attr.arkavo.com/clearance/<level>` (defaults to "internal")
///
/// Producers may construct their own policy with [`PolicyBuilder`] for
/// tighter or looser controls; this is a sensible default that matches
/// the spec example.
pub fn swarmkit_orchestrator_policy(clearance: Option<&str>) -> Result<Policy, TdfError> {
    let clearance = clearance.unwrap_or("internal");
    PolicyBuilder::new()
        .attribute_single("https://attr.arkavo.com/role", "orchestrator")
        .attribute_single("https://attr.arkavo.com/clearance", clearance)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_swarmkit::parse_yaml;
    use arkavo_tdf::testing::MockTdfService;
    use arkavo_test_macros::spec;

    const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "tdf-roundtrip-kit"
  version: "0.1.0"
  authors: [{did: "did:web:example.com"}]
  created: "2026-05-01T00:00:00Z"
  expires: "2026-05-30T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "exercise TDF envelope round-trip"
  success_criteria: ["wrapped"]
inputs: []
deliverables: [{name: "out", type: "json"}]
roles:
  - id: "worker"
    role_type: "specialist"
    agent_provisioning: {}
coordination:
  topology: "hub-spoke"
  routing: {strategy: "static"}
constraints:
  global_budget: {max_wallclock_seconds: 60, max_total_tokens: 8000, max_cost_usd: 0.05}
  data_classifications: ["public"]
  network: {egress_allowed: false, egress_allowlist: []}
completion:
  rules: ["all deliverables present"]
  on_failure: "abort"
  max_retries: 0
provenance:
  signatures: [{signer_did: "did:web:example.com", algorithm: "ed25519", signature: "AAA"}]
"#;

    fn policy() -> Policy {
        swarmkit_orchestrator_policy(None).unwrap()
    }

    #[spec("SK-050")]
    #[tokio::test]
    async fn wrap_then_unwrap_round_trips_manifest() {
        let manifest = parse_yaml(KIT).expect("parse manifest");
        let svc = MockTdfService::new(0xA5);
        let pol = policy();

        let tdf = wrap_manifest(&manifest, &svc, &pol).await.expect("wrap");
        let recovered = unwrap_manifest(&tdf, &svc).await.expect("unwrap");

        assert_eq!(manifest, recovered);
    }

    #[spec("SK-050")]
    #[tokio::test]
    async fn wrapped_payload_is_deterministic_for_same_input() {
        // Canonical-form serialization makes wrap output identical for
        // identical inputs (modulo any non-deterministic IV the encryptor
        // injects — MockTdfService has a deterministic XOR so the entire
        // ciphertext matches).
        let manifest = parse_yaml(KIT).unwrap();
        let svc = MockTdfService::new(0x42);
        let pol = policy();

        let a = wrap_manifest(&manifest, &svc, &pol).await.unwrap();
        let b = wrap_manifest(&manifest, &svc, &pol).await.unwrap();
        assert_eq!(a.payload.value, b.payload.value);
    }

    #[spec("SK-051")]
    #[tokio::test]
    async fn unwrap_runs_cross_block_validation() {
        // A tampered ciphertext that decrypts to bogus JSON surfaces as
        // a Parse error from unwrap_manifest, not a silent corruption.
        let manifest = parse_yaml(KIT).unwrap();
        let svc = MockTdfService::new(0x11);
        let pol = policy();

        let mut tdf = wrap_manifest(&manifest, &svc, &pol).await.unwrap();
        // Truncate the ciphertext to force a parse failure post-decrypt.
        tdf.payload.value.truncate(20);

        let err = unwrap_manifest(&tdf, &svc).await.unwrap_err();
        assert!(
            matches!(
                err,
                TdfEnvelopeError::Parse(_)
                    | TdfEnvelopeError::Utf8(_)
                    | TdfEnvelopeError::Decrypt(_)
            ),
            "expected Parse / Utf8 / Decrypt error, got {err:?}"
        );
    }

    #[test]
    fn orchestrator_policy_has_required_attributes() {
        let pol = swarmkit_orchestrator_policy(Some("confidential")).unwrap();
        assert_eq!(pol.attributes.len(), 2);
        let role_attr = pol
            .attributes
            .iter()
            .find(|a| a.attribute.contains("/role"))
            .expect("role attribute present");
        assert_eq!(role_attr.values, vec!["orchestrator".to_string()]);
        let clearance = pol
            .attributes
            .iter()
            .find(|a| a.attribute.contains("/clearance"))
            .expect("clearance attribute present");
        assert_eq!(clearance.values, vec!["confidential".to_string()]);
    }
}
