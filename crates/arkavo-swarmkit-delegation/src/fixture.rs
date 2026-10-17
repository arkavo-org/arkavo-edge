//! A two-role kit and keys for tests.

// Shared by the crate's modules only; plain `pub` here would trip
// `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use arkavo_crypto::AgentKeypair;
use arkavo_swarmkit::{Manifest, RoleSpec};
use chrono::{DateTime, TimeDelta, Utc};
use uuid::Uuid;

use crate::envelope::{Delegation, SignedDelegation};

pub(crate) const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: "blake3:M2WA9J8rOVUzfx-bbCkWGgNncM_q5L57CP6hg3P-FRs"
  name: image-swarm
  version: 0.1.0
  authors: [{did: "did:web:local.arkavo.dev"}]
  created: "2026-10-16T00:00:00Z"
  expires: "2027-01-14T00:00:00Z"
  nonce: "ZgOj5i5lHm-ZxfrCwEr5rQ"
objective:
  goal: Make game art on request
roles:
  - id: artist
    role_type: operator
    description: Generates game art on this Mac
    agent_provisioning:
      model: {family: gemma, size: E2B, backend: llama.cpp}
      inference: {temperature: 0.2}
    skills:
      - id: skill:identity
        version: 0.1.0
        source: inline
        payload: {name: identity, description: System identity, instructions: Make the images people ask for.}
    mcp_tools:
      - {server: arkavo, auth: none, tools: [generate_image, image_generation]}
    handoffs: [{to: writer, on: always}]
  - id: writer
    role_type: operator
    agent_provisioning:
      model: {family: gemma, size: E2B}
    mcp_tools:
      - {server: arkavo, auth: none, tools: [generate_image]}
coordination: {topology: hub-spoke, protocol: a2a-jsonrpc-2.0, routing: {strategy: static}}
constraints:
  global_budget: {max_wallclock_seconds: 1800, max_total_tokens: 100000, max_cost_usd: 0.0}
  network: {egress_allowed: false}
completion: {rules: [objective addressed], on_failure: abort, max_retries: 0}
provenance: {signatures: []}
"#;

pub(crate) fn manifest() -> Manifest {
    arkavo_swarmkit::parse_yaml(KIT).expect("fixture kit parses")
}

pub(crate) fn role(manifest: &Manifest, id: &str) -> RoleSpec {
    manifest
        .roles
        .iter()
        .find(|r| r.id == id)
        .cloned()
        .expect("fixture role")
}

pub(crate) fn did(keypair: &AgentKeypair) -> String {
    keypair.public_key().to_did_key()
}

/// `role` of the fixture kit for `specialist`, signed by `orchestrator`,
/// issued at `issued` for a day.
pub(crate) fn envelope(
    orchestrator: &AgentKeypair,
    specialist: &str,
    role_id: &str,
    issued: DateTime<Utc>,
) -> SignedDelegation {
    let manifest = manifest();
    Delegation::for_role(
        &manifest,
        &role(&manifest, role_id),
        did(orchestrator),
        specialist.to_string(),
        Uuid::new_v4(),
        issued,
        TimeDelta::days(1),
    )
    .expect("fixture envelope builds")
    .sign(orchestrator)
    .expect("fixture envelope signs")
}
