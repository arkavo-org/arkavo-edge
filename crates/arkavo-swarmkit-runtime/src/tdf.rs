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
//! tree. Per-role TDF policy construction (spec §6.4) is implemented
//! via [`role_policy`] / [`role_policies`]. Out of scope for now:
//! KAS-gated decryption enforcement (spec §6.3 orchestrator gate),
//! `.swarmkit.tdf` file-format serialization, and `.tdf`-aware
//! auto-launch — those land in subsequent slices.
//!
//! Agent identity binding: every policy builder accepts an optional
//! agent DID that is added to the policy's dissemination list. Callers
//! supply the DID explicitly, or use the
//! [`swarmkit_orchestrator_policy_for_current_agent`] /
//! [`role_policies_for_current_agent`] async helpers to load the
//! locally-stored token from `arkavo-agent-auth` and bind to it.

use std::collections::HashMap;

use arkavo_swarmkit::{Manifest, ParseError, TdfAttributeReleasePolicy, parse_json};
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
    #[error("build policy: {0}")]
    Policy(TdfError),
    #[error("load agent identity: {0}")]
    Identity(String),
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
/// When `orchestrator_did` is supplied, it is added to the policy's
/// dissemination list — matching the spec §6.3 example
/// `"dissem": ["did:web:orchestrator.arkavo.net"]` — so only the named
/// DID's holder can request the wrapped key from the KAS.
///
/// Producers may construct their own policy with [`PolicyBuilder`] for
/// tighter or looser controls; this is a sensible default that matches
/// the spec example. Pair with [`swarmkit_orchestrator_policy_for_current_agent`]
/// when you want the policy bound to the locally-stored agent identity.
pub fn swarmkit_orchestrator_policy(
    clearance: Option<&str>,
    orchestrator_did: Option<&str>,
) -> Result<Policy, TdfError> {
    let clearance = clearance.unwrap_or("internal");
    let mut builder = PolicyBuilder::new()
        .attribute_single("https://attr.arkavo.com/role", "orchestrator")
        .attribute_single("https://attr.arkavo.com/clearance", clearance);
    if let Some(did) = orchestrator_did {
        builder = builder.add_dissemination(did);
    }
    builder.build()
}

/// Load the running agent's DID from `arkavo-agent-auth`'s `StoredToken`
/// and emit a §6.3 orchestrator-gate policy bound to it.
///
/// Returns `Identity` error when no token is on disk or when the token
/// is expired. Use [`swarmkit_orchestrator_policy`] with an explicit DID
/// when you want to build a policy without the side-effect of disk I/O.
pub async fn swarmkit_orchestrator_policy_for_current_agent(
    clearance: Option<&str>,
) -> Result<Policy, TdfEnvelopeError> {
    let did = load_current_agent_did().await?;
    swarmkit_orchestrator_policy(clearance, Some(&did)).map_err(TdfEnvelopeError::Policy)
}

/// Convert a SwarmKit role's `tdf_attribute_release_policy` block into a
/// runnable [`Policy`] per spec §6.4.
///
/// The orchestrator uses the returned policy to re-wrap data objects
/// before passing them to the specialist. Each attribute string in the
/// manifest is treated as `<fqn>/<value>` — the prefix up to the last
/// `/` becomes the OpenTDF attribute FQN and the suffix becomes the
/// value. Example:
///
/// * `"https://attr.arkavo.com/role/planner"` →
///   FQN `"https://attr.arkavo.com/role"` + value `"planner"`
///
/// Attributes with the same FQN are merged (one OpenTDF [`Attribute`]
/// with multiple values), so a manifest carrying
/// `["https://.../role/planner", "https://.../role/critic"]` produces a
/// single role attribute with both values.
///
/// `agent_did`, when supplied, is added to the policy's dissemination
/// list so the policy binds to that specific specialist identity. Pass
/// `None` for an identity-agnostic policy (any holder of the required
/// attributes can decrypt).
///
/// The manifest's `rule` field (`AllOf` / `AnyOf` / `Hierarchy`) is
/// metadata that the OpenTDF policy structure does not carry directly;
/// rule semantics are evaluated KAS-side at attribute-definition time.
/// Callers that need to honor `rule` differently should pass it through
/// alongside the policy.
pub fn role_policy(
    role_id: &str,
    arp: &TdfAttributeReleasePolicy,
    agent_did: Option<&str>,
) -> Result<Policy, TdfError> {
    let mut by_fqn: Vec<(String, Vec<String>)> = Vec::new();
    for attr in &arp.attributes {
        let (fqn, value) = split_attribute(attr).ok_or_else(|| {
            TdfError::Policy(format!(
                "role {role_id:?}: attribute {attr:?} is not in the form '<fqn>/<value>'"
            ))
        })?;
        match by_fqn.iter_mut().find(|(f, _)| f == &fqn) {
            Some((_, vs)) => {
                if !vs.iter().any(|v| v == &value) {
                    vs.push(value);
                }
            }
            None => by_fqn.push((fqn, vec![value])),
        }
    }
    let mut builder = PolicyBuilder::new().id(&format!("swarmkit:role:{role_id}"));
    for (fqn, values) in &by_fqn {
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        builder = builder.attribute(fqn, &refs);
    }
    if let Some(did) = agent_did {
        builder = builder.add_dissemination(did);
    }
    builder.build()
}

/// Build per-role policies for every role in the manifest that declares
/// a `tdf_attribute_release_policy` block. Roles without one are
/// omitted from the returned map — the caller decides whether to treat
/// that as an error or to fall back to a permissive default.
///
/// `did_for` is a per-role lookup that the caller controls. Typical
/// implementations:
///
/// * `|_| None` — identity-agnostic policies (attribute-only).
/// * `|_| Some(orchestrator_did)` — bind every role's data to the
///   orchestrator's own DID, since the orchestrator runs each role
///   in-process for now (no specialist subprocess yet).
/// * `|role_id| specialist_dids.get(role_id).cloned()` — once
///   specialists are provisioned, look up each one's DID.
pub fn role_policies<F>(
    manifest: &Manifest,
    mut did_for: F,
) -> Result<HashMap<String, Policy>, TdfError>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut out = HashMap::new();
    for role in &manifest.roles {
        if let Some(arp) = &role.tdf_attribute_release_policy {
            let did = did_for(&role.id);
            let pol = role_policy(&role.id, arp, did.as_deref())?;
            out.insert(role.id.clone(), pol);
        }
    }
    Ok(out)
}

/// Convenience: build per-role policies, binding every role's data
/// dissemination to the locally-stored agent identity from
/// `arkavo-agent-auth`. Used when the orchestrator runs every role in
/// its own process and decryption must scope to that single DID.
pub async fn role_policies_for_current_agent(
    manifest: &Manifest,
) -> Result<HashMap<String, Policy>, TdfEnvelopeError> {
    let did = load_current_agent_did().await?;
    role_policies(manifest, |_| Some(did.clone())).map_err(TdfEnvelopeError::Policy)
}

/// Load the running agent's DID from `arkavo-agent-auth`'s on-disk
/// `StoredToken`. Returns `Identity` error when no token exists or the
/// token is expired.
pub async fn load_current_agent_did() -> Result<String, TdfEnvelopeError> {
    let stored = arkavo_agent_auth::load_token()
        .await
        .map_err(|e| TdfEnvelopeError::Identity(e.to_string()))?
        .ok_or_else(|| {
            TdfEnvelopeError::Identity(
                "no StoredToken on disk; agent must register and authenticate first".to_string(),
            )
        })?;
    if stored.is_expired() {
        return Err(TdfEnvelopeError::Identity(format!(
            "StoredToken for {did:?} expired at {expires}",
            did = stored.did,
            expires = stored.expires_at
        )));
    }
    Ok(stored.did)
}

/// Split a SwarmKit attribute string at the last `/` into (fqn, value).
/// Returns `None` if the input has no `/`, has nothing after the final
/// `/`, or the prefix is not an http(s) URL — those would all fail
/// `PolicyBuilder` validation downstream anyway.
fn split_attribute(attr: &str) -> Option<(String, String)> {
    let idx = attr.rfind('/')?;
    if idx == 0 || idx == attr.len() - 1 {
        return None;
    }
    let (fqn, rest) = attr.split_at(idx);
    if !fqn.starts_with("http://") && !fqn.starts_with("https://") {
        return None;
    }
    let value = &rest[1..];
    if value.is_empty() {
        return None;
    }
    Some((fqn.to_string(), value.to_string()))
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
        swarmkit_orchestrator_policy(None, None).unwrap()
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

    #[spec("SK-052")]
    #[test]
    fn orchestrator_policy_has_required_attributes() {
        let pol = swarmkit_orchestrator_policy(Some("confidential"), None).unwrap();
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
        assert!(pol.dissemination.is_empty());
    }

    #[spec("SK-052")]
    #[test]
    fn orchestrator_policy_binds_to_supplied_did() {
        let pol =
            swarmkit_orchestrator_policy(None, Some("did:web:orchestrator.arkavo.net")).unwrap();
        assert_eq!(
            pol.dissemination,
            vec!["did:web:orchestrator.arkavo.net".to_string()]
        );
    }

    #[spec("SK-053")]
    #[test]
    fn role_policy_splits_attributes_at_last_slash() {
        use arkavo_swarmkit::ArpRule;
        let arp = TdfAttributeReleasePolicy {
            attributes: vec![
                "https://attr.arkavo.com/role/planner".to_string(),
                "https://attr.arkavo.com/clearance/internal".to_string(),
            ],
            rule: ArpRule::AllOf,
        };
        let pol = role_policy("planner-1", &arp, None).unwrap();
        assert_eq!(pol.attributes.len(), 2);
        let role = pol
            .attributes
            .iter()
            .find(|a| a.attribute == "https://attr.arkavo.com/role")
            .unwrap();
        assert_eq!(role.values, vec!["planner".to_string()]);
        let clearance = pol
            .attributes
            .iter()
            .find(|a| a.attribute == "https://attr.arkavo.com/clearance")
            .unwrap();
        assert_eq!(clearance.values, vec!["internal".to_string()]);
        assert_eq!(pol.id.as_deref(), Some("swarmkit:role:planner-1"));
    }

    #[spec("SK-053")]
    #[test]
    fn role_policy_merges_repeated_fqns() {
        use arkavo_swarmkit::ArpRule;
        let arp = TdfAttributeReleasePolicy {
            attributes: vec![
                "https://attr.arkavo.com/role/planner".to_string(),
                "https://attr.arkavo.com/role/critic".to_string(),
            ],
            rule: ArpRule::AnyOf,
        };
        let pol = role_policy("multi", &arp, None).unwrap();
        assert_eq!(pol.attributes.len(), 1);
        let mut values = pol.attributes[0].values.clone();
        values.sort();
        assert_eq!(values, vec!["critic".to_string(), "planner".to_string()]);
    }

    #[spec("SK-053")]
    #[test]
    fn role_policy_binds_to_agent_did_via_dissemination() {
        use arkavo_swarmkit::ArpRule;
        let arp = TdfAttributeReleasePolicy {
            attributes: vec!["https://attr.arkavo.com/clearance/public".to_string()],
            rule: ArpRule::AllOf,
        };
        let pol = role_policy("worker", &arp, Some("did:web:specialist.example.com")).unwrap();
        assert_eq!(
            pol.dissemination,
            vec!["did:web:specialist.example.com".to_string()]
        );
    }

    #[spec("SK-054")]
    #[test]
    fn role_policy_rejects_malformed_attribute() {
        use arkavo_swarmkit::ArpRule;
        let arp = TdfAttributeReleasePolicy {
            // Missing a slash → cannot split into (fqn, value).
            attributes: vec!["malformed".to_string()],
            rule: ArpRule::AllOf,
        };
        let err = role_policy("r1", &arp, None).unwrap_err();
        assert!(matches!(err, TdfError::Policy(_)));
    }

    #[spec("SK-054")]
    #[test]
    fn role_policies_extracts_only_roles_with_arp() {
        // Reuse KIT (single role with arp omitted via empty agent_provisioning),
        // build a manifest where only one of two roles has an ARP block.
        let kit_with_two = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "two-role-policy-kit"
  version: "0.1.0"
  authors: [{did: "did:web:example.com"}]
  created: "2026-05-01T00:00:00Z"
  expires: "2026-05-30T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "exercise role_policies"
  success_criteria: ["done"]
inputs: []
deliverables: [{name: "out", type: "json"}]
roles:
  - id: "with-arp"
    role_type: "specialist"
    agent_provisioning: {}
    tdf_attribute_release_policy:
      attributes: ["https://attr.arkavo.com/clearance/public"]
      rule: "allOf"
  - id: "without-arp"
    role_type: "critic"
    agent_provisioning: {}
coordination:
  topology: "pipeline"
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
        let m = parse_yaml(kit_with_two).unwrap();
        let pols = role_policies(&m, |_| None).unwrap();
        assert_eq!(pols.len(), 1);
        assert!(pols.contains_key("with-arp"));
        assert!(!pols.contains_key("without-arp"));
    }

    #[spec("SK-054")]
    #[test]
    fn role_policies_did_lookup_per_role() {
        let kit_with_two = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "did-lookup-kit"
  version: "0.1.0"
  authors: [{did: "did:web:example.com"}]
  created: "2026-05-01T00:00:00Z"
  expires: "2026-05-30T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "per-role DID binding"
  success_criteria: ["done"]
inputs: []
deliverables: [{name: "out", type: "json"}]
roles:
  - id: "alpha"
    role_type: "specialist"
    agent_provisioning: {}
    tdf_attribute_release_policy:
      attributes: ["https://attr.arkavo.com/clearance/public"]
      rule: "allOf"
  - id: "beta"
    role_type: "specialist"
    agent_provisioning: {}
    tdf_attribute_release_policy:
      attributes: ["https://attr.arkavo.com/clearance/public"]
      rule: "allOf"
coordination:
  topology: "pipeline"
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
        let m = parse_yaml(kit_with_two).unwrap();
        let pols =
            role_policies(&m, |role_id| Some(format!("did:web:{role_id}.example.com"))).unwrap();
        assert_eq!(
            pols["alpha"].dissemination,
            vec!["did:web:alpha.example.com".to_string()]
        );
        assert_eq!(
            pols["beta"].dissemination,
            vec!["did:web:beta.example.com".to_string()]
        );
    }
}
