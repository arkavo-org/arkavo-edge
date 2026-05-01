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
//! via [`role_policy`] / [`role_policies`]. The `.swarmkit.tdf` file
//! format is implemented via [`write_kit_tdf`] / [`read_kit_tdf`] and
//! their path-based / one-shot variants. KAS-gated unwrap (spec §6.3)
//! is implemented via [`unwrap_manifest_kas_gated`] with embedded-policy
//! verification + KAS health check before decrypt. Out of scope for
//! now: `.tdf`-aware auto-launch — that lands in the final slice.
//!
//! Agent identity binding: every policy builder accepts an optional
//! agent DID that is added to the policy's dissemination list. Callers
//! supply the DID explicitly, or use the
//! [`swarmkit_orchestrator_policy_for_current_agent`] /
//! [`role_policies_for_current_agent`] async helpers to load the
//! locally-stored token from `arkavo-agent-auth` and bind to it.
//!
//! KAS gating: [`unwrap_manifest_kas_gated`] adds an explicit KAS health
//! check + embedded-policy verification before invoking the underlying
//! decrypt path. The opentdf-rs decryptor performs its own KAS rewrap
//! internally; the SwarmKit gate complements that with fail-fast checks
//! (KAS unreachable, policy attributes don't match what the orchestrator
//! requires) so the orchestrator never attempts a decrypt that's
//! guaranteed to be denied or that's against an unexpected policy.
//!
//! P2P transport: this module covers the **encryption / policy plane
//! only**. For peer-to-peer distribution of `.swarmkit.tdf` blobs
//! (staging, fetching, content addressing), compose with the existing
//! [`arkavo-tdf-iroh`] crate's `IrohTransport` — do NOT re-implement
//! transport here.

use std::collections::HashMap;

use arkavo_swarmkit::{Manifest, ParseError, TdfAttributeReleasePolicy, parse_json};
use arkavo_tdf::{
    KasClient, Policy, PolicyBuilder, TdfDecryptor, TdfEncryptor, TdfError, TdfManifest,
};
use base64::{Engine, engine::general_purpose::STANDARD as B64};

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
    #[error(".swarmkit.tdf I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("KAS unavailable: {0}")]
    KasUnavailable(String),
    #[error("embedded policy could not be parsed: {0}")]
    EmbeddedPolicy(String),
    #[error("embedded policy is missing required attribute {missing:?} (policy carries {found:?})")]
    PolicyMismatch {
        missing: Vec<String>,
        found: Vec<String>,
    },
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

// --- KAS-gated unwrap ---------------------------------------------------

/// Extract the policy embedded in a [`TdfManifest`] (spec §6.1
/// `encryptionInformation.policy`, base64-encoded JSON) and parse it
/// back into a [`Policy`]. Useful for inspecting what attributes a
/// distributed `.swarmkit.tdf` actually requires before attempting to
/// unwrap it.
pub fn extract_embedded_policy(tdf: &TdfManifest) -> Result<Policy, TdfEnvelopeError> {
    let raw = &tdf.encryption_information.policy;
    let bytes = B64
        .decode(raw)
        .map_err(|e| TdfEnvelopeError::EmbeddedPolicy(format!("base64: {e}")))?;
    let policy: Policy = serde_json::from_slice(&bytes)
        .map_err(|e| TdfEnvelopeError::EmbeddedPolicy(format!("json: {e}")))?;
    Ok(policy)
}

/// Verify that the embedded policy on a [`TdfManifest`] requires every
/// attribute FQN listed in `required_fqns`. Returns
/// [`TdfEnvelopeError::PolicyMismatch`] when one or more are missing.
///
/// Use this *before* attempting to unwrap a kit when the orchestrator
/// has expectations about which attributes the wrapped content was
/// gated on (e.g. a producer that always wraps under
/// `https://attr.arkavo.com/role/orchestrator` can refuse kits that
/// don't carry that requirement, distinguishing a misrouted kit from a
/// genuine KAS denial at decrypt time).
pub fn verify_embedded_policy_requires(
    tdf: &TdfManifest,
    required_fqns: &[&str],
) -> Result<Policy, TdfEnvelopeError> {
    let policy = extract_embedded_policy(tdf)?;
    let found: Vec<String> = policy
        .attributes
        .iter()
        .map(|a| a.attribute.clone())
        .collect();
    let missing: Vec<String> = required_fqns
        .iter()
        .filter(|fqn| !found.iter().any(|f| f == *fqn))
        .map(|s| (*s).to_string())
        .collect();
    if !missing.is_empty() {
        return Err(TdfEnvelopeError::PolicyMismatch { missing, found });
    }
    Ok(policy)
}

/// KAS-gated unwrap. Adds two pre-flight checks the bare
/// [`unwrap_manifest`] path doesn't perform:
///
/// 1. **KAS health**: if `kas.health_check()` returns `Ok(false)` or
///    errors, fail fast with [`TdfEnvelopeError::KasUnavailable`]
///    instead of timing out inside the decryptor.
/// 2. **Policy verification**: extract the policy embedded in the TDF
///    and confirm it requires every FQN in `required_fqns`. Surfaces
///    [`TdfEnvelopeError::PolicyMismatch`] for misrouted or
///    misconfigured kits before any KAS round-trip.
///
/// Pass an empty `required_fqns` slice to skip the policy check and
/// rely on KAS-side enforcement only. Pass
/// `&["https://attr.arkavo.com/role"]` (or similar) for the
/// orchestrator-gated baseline.
///
/// The KAS rewrap itself happens inside the underlying [`TdfDecryptor`]
/// implementation (opentdf-rs talks to KAS during decrypt). The KAS
/// client passed here is used only for the health check; SwarmKit
/// doesn't currently re-implement the rewrap protocol.
pub async fn unwrap_manifest_kas_gated<D, K>(
    tdf: &TdfManifest,
    decryptor: &D,
    kas: &K,
    required_fqns: &[&str],
) -> Result<Manifest, TdfEnvelopeError>
where
    D: TdfDecryptor,
    K: KasClient,
{
    // 1. Health gate — fail fast if KAS is unreachable so we don't
    //    proceed into the decryptor's slower retry/timeout paths.
    match kas.health_check().await {
        Ok(true) => {}
        Ok(false) => {
            return Err(TdfEnvelopeError::KasUnavailable(
                "health_check returned false".to_string(),
            ));
        }
        Err(e) => return Err(TdfEnvelopeError::KasUnavailable(e.to_string())),
    }

    // 2. Embedded-policy verification — surface a distinct error when
    //    the kit's policy doesn't carry the attributes the orchestrator
    //    requires. This separates "I won't even try" from "I tried and
    //    KAS denied me."
    if !required_fqns.is_empty() {
        verify_embedded_policy_requires(tdf, required_fqns)?;
    }

    // 3. Decrypt + parse + validate. KAS rewrap happens inside.
    unwrap_manifest(tdf, decryptor).await
}

/// Convenience: KAS-gated unwrap that also reads from a path.
pub async fn unwrap_manifest_from_path_kas_gated<D, K, P>(
    decryptor: &D,
    kas: &K,
    path: P,
    required_fqns: &[&str],
) -> Result<Manifest, TdfEnvelopeError>
where
    D: TdfDecryptor,
    K: KasClient,
    P: AsRef<std::path::Path>,
{
    let tdf = read_kit_tdf_from_path(path)?;
    unwrap_manifest_kas_gated(&tdf, decryptor, kas, required_fqns).await
}

// --- .swarmkit.tdf file format -----------------------------------------

/// Recommended file extension for SwarmKit TDF envelopes on disk.
pub const SWARMKIT_TDF_EXTENSION: &str = "swarmkit.tdf";

/// Write a [`TdfManifest`] to disk as a `.swarmkit.tdf` file.
///
/// The on-disk format is the canonical JSON encoding of the
/// [`TdfManifest`] struct itself — encryption metadata
/// (`encryptionInformation`) and the inline base64 ciphertext
/// (`payload`) live in a single self-contained JSON document. This is
/// strictly equivalent to the in-memory `TdfManifest` and round-trips
/// losslessly via [`read_kit_tdf`].
///
/// This is not the OpenTDF ZIP container format — that's a future
/// interop slice when we need to exchange `.tdf` files with external
/// OpenTDF tooling. For SwarmKit-internal storage and transport, the
/// JSON form is simpler, scriptable, and sufficient.
pub fn write_kit_tdf<W: std::io::Write>(
    tdf: &TdfManifest,
    mut writer: W,
) -> Result<(), TdfEnvelopeError> {
    let json = serde_json::to_vec_pretty(tdf)?;
    writer.write_all(&json)?;
    writer.write_all(b"\n")?;
    Ok(())
}

/// Read a [`TdfManifest`] from a `.swarmkit.tdf` file written by
/// [`write_kit_tdf`].
pub fn read_kit_tdf<R: std::io::Read>(reader: R) -> Result<TdfManifest, TdfEnvelopeError> {
    let tdf: TdfManifest = serde_json::from_reader(reader)?;
    Ok(tdf)
}

/// Convenience: write a `.swarmkit.tdf` to a path, creating or
/// truncating the file as needed.
pub fn write_kit_tdf_to_path<P: AsRef<std::path::Path>>(
    tdf: &TdfManifest,
    path: P,
) -> Result<(), TdfEnvelopeError> {
    let file = std::fs::File::create(path.as_ref())?;
    write_kit_tdf(tdf, std::io::BufWriter::new(file))
}

/// Convenience: read a `.swarmkit.tdf` from a path.
pub fn read_kit_tdf_from_path<P: AsRef<std::path::Path>>(
    path: P,
) -> Result<TdfManifest, TdfEnvelopeError> {
    let file = std::fs::File::open(path.as_ref())?;
    read_kit_tdf(std::io::BufReader::new(file))
}

/// One-shot producer flow: encrypt a manifest and write it to a
/// `.swarmkit.tdf` file in a single call.
pub async fn wrap_manifest_to_path<E, P>(
    manifest: &Manifest,
    encryptor: &E,
    policy: &Policy,
    path: P,
) -> Result<(), TdfEnvelopeError>
where
    E: TdfEncryptor,
    P: AsRef<std::path::Path>,
{
    let tdf = wrap_manifest(manifest, encryptor, policy).await?;
    write_kit_tdf_to_path(&tdf, path)
}

/// One-shot consumer flow: read a `.swarmkit.tdf` file and decrypt
/// straight to a validated `Manifest`.
pub async fn unwrap_manifest_from_path<D, P>(
    decryptor: &D,
    path: P,
) -> Result<Manifest, TdfEnvelopeError>
where
    D: TdfDecryptor,
    P: AsRef<std::path::Path>,
{
    let tdf = read_kit_tdf_from_path(path)?;
    unwrap_manifest(&tdf, decryptor).await
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

    // --- KAS-gated unwrap ------------------------------------------------

    use arkavo_tdf::testing::MockKasClient;

    #[spec("SK-058")]
    #[tokio::test]
    async fn extract_embedded_policy_decodes_base64_json() {
        let manifest = parse_yaml(KIT).unwrap();
        let svc = MockTdfService::new(0xC3);
        let pol = swarmkit_orchestrator_policy(None, Some("did:web:gateway.example.com")).unwrap();
        let tdf = wrap_manifest(&manifest, &svc, &pol).await.unwrap();

        let recovered = extract_embedded_policy(&tdf).expect("decode policy");
        assert_eq!(recovered.attributes.len(), 2);
        assert!(
            recovered
                .attributes
                .iter()
                .any(|a| a.attribute.contains("/role"))
        );
        assert_eq!(
            recovered.dissemination,
            vec!["did:web:gateway.example.com".to_string()]
        );
    }

    #[spec("SK-058")]
    #[tokio::test]
    async fn verify_embedded_policy_requires_passes_for_matching_attrs() {
        let manifest = parse_yaml(KIT).unwrap();
        let svc = MockTdfService::new(0xC3);
        let pol = swarmkit_orchestrator_policy(None, None).unwrap();
        let tdf = wrap_manifest(&manifest, &svc, &pol).await.unwrap();

        verify_embedded_policy_requires(&tdf, &["https://attr.arkavo.com/role"])
            .expect("policy carries the role attribute");
    }

    #[spec("SK-058")]
    #[tokio::test]
    async fn verify_embedded_policy_rejects_missing_attr() {
        let manifest = parse_yaml(KIT).unwrap();
        let svc = MockTdfService::new(0xC3);
        let pol = swarmkit_orchestrator_policy(None, None).unwrap();
        let tdf = wrap_manifest(&manifest, &svc, &pol).await.unwrap();

        let err = verify_embedded_policy_requires(
            &tdf,
            &[
                "https://attr.arkavo.com/role",
                "https://attr.arkavo.com/region",
            ],
        )
        .unwrap_err();
        match err {
            TdfEnvelopeError::PolicyMismatch { missing, found } => {
                assert_eq!(missing, vec!["https://attr.arkavo.com/region".to_string()]);
                assert!(found.iter().any(|a| a == "https://attr.arkavo.com/role"));
            }
            other => panic!("expected PolicyMismatch, got {other:?}"),
        }
    }

    #[spec("SK-059")]
    #[tokio::test]
    async fn unwrap_manifest_kas_gated_succeeds_when_kas_healthy_and_policy_matches() {
        let manifest = parse_yaml(KIT).unwrap();
        let svc = MockTdfService::new(0xD1);
        let pol = swarmkit_orchestrator_policy(None, None).unwrap();
        let tdf = wrap_manifest(&manifest, &svc, &pol).await.unwrap();

        let kas = MockKasClient::new();
        let recovered =
            unwrap_manifest_kas_gated(&tdf, &svc, &kas, &["https://attr.arkavo.com/role"])
                .await
                .expect("kas-gated unwrap");
        assert_eq!(manifest, recovered);
    }

    #[spec("SK-060")]
    #[tokio::test]
    async fn unwrap_manifest_kas_gated_fails_fast_when_kas_unhealthy() {
        let manifest = parse_yaml(KIT).unwrap();
        let svc = MockTdfService::new(0xD2);
        let pol = swarmkit_orchestrator_policy(None, None).unwrap();
        let tdf = wrap_manifest(&manifest, &svc, &pol).await.unwrap();

        let kas = MockKasClient::new();
        kas.set_healthy(false);

        let err = unwrap_manifest_kas_gated(&tdf, &svc, &kas, &[])
            .await
            .unwrap_err();
        assert!(matches!(err, TdfEnvelopeError::KasUnavailable(_)));
    }

    #[spec("SK-060")]
    #[tokio::test]
    async fn unwrap_manifest_kas_gated_surfaces_policy_mismatch_before_decrypt() {
        let manifest = parse_yaml(KIT).unwrap();
        let svc = MockTdfService::new(0xD3);
        let pol = swarmkit_orchestrator_policy(None, None).unwrap();
        let tdf = wrap_manifest(&manifest, &svc, &pol).await.unwrap();

        let kas = MockKasClient::new();
        let err = unwrap_manifest_kas_gated(&tdf, &svc, &kas, &["https://attr.arkavo.com/region"])
            .await
            .unwrap_err();
        assert!(matches!(err, TdfEnvelopeError::PolicyMismatch { .. }));
    }

    // --- .swarmkit.tdf file format ---------------------------------------

    #[spec("SK-055")]
    #[tokio::test]
    async fn write_kit_tdf_round_trips_through_reader() {
        let manifest = parse_yaml(KIT).unwrap();
        let svc = MockTdfService::new(0x33);
        let pol = policy();
        let tdf = wrap_manifest(&manifest, &svc, &pol).await.unwrap();

        let mut buf: Vec<u8> = Vec::new();
        write_kit_tdf(&tdf, &mut buf).expect("write");
        let recovered = read_kit_tdf(&buf[..]).expect("read");

        // The TdfManifest is serde-equal across the round trip.
        let a = serde_json::to_value(&tdf).unwrap();
        let b = serde_json::to_value(&recovered).unwrap();
        assert_eq!(a, b);
    }

    #[spec("SK-056")]
    #[tokio::test]
    async fn wrap_unwrap_via_path_round_trips_manifest() {
        let manifest = parse_yaml(KIT).unwrap();
        let svc = MockTdfService::new(0x77);
        let pol = policy();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.swarmkit.tdf");

        wrap_manifest_to_path(&manifest, &svc, &pol, &path)
            .await
            .expect("wrap to path");
        let recovered = unwrap_manifest_from_path(&svc, &path)
            .await
            .expect("unwrap from path");

        assert_eq!(manifest, recovered);
    }

    #[spec("SK-057")]
    #[tokio::test]
    async fn unwrap_from_path_surfaces_io_error_for_missing_file() {
        let svc = MockTdfService::new(0x44);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.swarmkit.tdf");

        let err = unwrap_manifest_from_path(&svc, &path).await.unwrap_err();
        assert!(matches!(err, TdfEnvelopeError::Io(_)));
    }

    #[spec("SK-057")]
    #[test]
    fn read_kit_tdf_rejects_non_json() {
        let bogus = b"this is not a tdf manifest";
        let err = read_kit_tdf(&bogus[..]).unwrap_err();
        assert!(matches!(err, TdfEnvelopeError::Serialize(_)));
    }

    // --- end file format -------------------------------------------------

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
