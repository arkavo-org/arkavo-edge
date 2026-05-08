//! SwarmFlight runtime — the bridge from a parsed SwarmKit manifest to per-role
//! Agent Runtime Policy (ARP) instances.
//!
//! Per the SwarmKit specification §1.2 and §5, a SwarmKit's `agent_provisioning`
//! block sets the *initial conditions* for each specialist; the companion ARP
//! specification governs how each running specialist *adapts* over time. This
//! crate executes the handoff: when a SwarmFlight launches, it constructs a
//! per-role `ArpRuntime` (PolicyCache + AdaptationEngine + DecisionTrace) that
//! is keyed to the role and isolated from the other roles' runtimes.
//!
//! Out of scope for this crate: TDF encryption, KAS unwrap, A2A wire envelopes,
//! specialist-process spawning, model loading, MCP grant brokering. The
//! SwarmFlight here is a logical orchestration object, not a network actor.

pub mod derive;
pub mod flight;
pub mod skill_resolver;
#[cfg(feature = "tdf")]
pub mod tdf;

pub use derive::{DeriveOptions, derive_arp_for_role};
pub use flight::{FlightError, LaunchError, LaunchOptions, RoleRuntime, SwarmFlight};
pub use skill_resolver::{
    DidWebPublicKeyResolver, MockPublicKeyResolver, PublicKeyResolver, ResolveError, ResolvedSkill,
    ResolverConfig, SignedSkill, VerifyMode, resolve_skill, sign_skill_content,
};

#[cfg(feature = "tdf")]
pub use tdf::{
    SWARMKIT_TDF_EXTENSION, TdfEnvelopeError, extract_embedded_policy, load_current_agent_did,
    read_kit_tdf, read_kit_tdf_from_path, role_policies, role_policies_for_current_agent,
    role_policy, swarmkit_orchestrator_policy, swarmkit_orchestrator_policy_for_current_agent,
    unwrap_manifest, unwrap_manifest_from_path, unwrap_manifest_from_path_kas_gated,
    unwrap_manifest_kas_gated, verify_embedded_policy_requires, wrap_manifest,
    wrap_manifest_to_path, write_kit_tdf, write_kit_tdf_to_path,
};

// Re-export the underlying TDF runtime types so downstream consumers
// (the AG-UI gateway's auto-launch path, the swarmkit demo binaries,
// etc.) can compose with our wrap/unwrap helpers without taking a
// direct dependency on arkavo-tdf. Stable surface for the long-lived
// integration branch.
#[cfg(feature = "tdf")]
pub use arkavo_tdf::{
    KasClient, OpenTdfConfig, OpenTdfService, Policy as TdfPolicy, TdfDecryptor, TdfEncryptor,
    TdfManifest,
};

/// Serialize a manifest to canonical JSON (sorted keys, no insignificant
/// whitespace) for hashing, signing, or TDF wrapping.
///
/// Distinct from `arkavo_swarmkit::canonical::kit_id_for` which strips
/// `kit.id` and `provenance.signatures` before hashing — this helper
/// preserves every field for round-trip reconstruction.
pub fn canonical_full_manifest(
    manifest: &arkavo_swarmkit::Manifest,
) -> Result<String, serde_json::Error> {
    let value = serde_json::to_value(manifest)?;
    Ok(arkavo_swarmkit::canonical_json(&value))
}

#[cfg(test)]
mod tests {
    use crate::{LaunchOptions, SwarmFlight};
    use arkavo_swarmkit::parse_yaml;
    use arkavo_test_macros::spec;

    const TINY_KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "Tiny Kit"
  version: "0.1.0"
  authors: [{did: "did:web:example.com"}]
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "trivial"
  success_criteria: ["done"]
inputs: []
deliverables:
  - {name: "out", type: "json"}
roles:
  - id: "worker"
    role_type: "specialist"
    agent_provisioning:
      model: {family: "qwen3", size: "3B", backend: "llama.cpp"}
      inference: {max_tokens: 512, temperature: 0.1, thinking: false}
      budget: {max_total_tokens: 4000, max_wallclock_ms: 30000}
      context: {persistence: "ephemeral"}
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

    #[spec("SK-010")]
    #[tokio::test]
    async fn launch_creates_per_role_runtime() {
        let manifest = parse_yaml(TINY_KIT).unwrap();
        let flight = SwarmFlight::launch(&manifest, LaunchOptions::default()).unwrap();
        let role = flight.role("worker").expect("worker role exists");
        assert_eq!(role.role_type(), "specialist");
    }
}
