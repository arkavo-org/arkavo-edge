//! Integration test mirroring the §12.1 single-role example from the SwarmKit spec.
//! Validates that the canonical-form spec example parses, validates, and round-trips
//! its kit.id under our BLAKE3 implementation.

use arkavo_swarmkit::{kit_id_for, parse_yaml};
use arkavo_test_macros::spec;

const SPEC_EXAMPLE_YAML: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "Markdown Linter"
  version: "1.0.0"
  description: "Lint markdown for style violations"
  authors:
    - did: "did:web:arkavo.net"
      name: "Arkavo"
  created: "2026-04-29T12:00:00Z"
  expires: "2026-07-28T12:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "Identify style violations in a markdown file"
  success_criteria:
    - "all violations enumerated"
    - "no false positives in test set"
inputs:
  - name: "doc"
    type: "text"
    required: true
deliverables:
  - name: "violations"
    type: "json"
roles:
  - id: "linter"
    role_type: "specialist"
    description: "Apply lint rules"
    agent_provisioning:
      model:
        family: "qwen3"
        size: "3B"
        quantization: "Q4_K_M"
        backend: "llama.cpp"
      inference:
        max_tokens: 512
        temperature: 0.1
        thinking: false
      budget:
        max_inference_calls: 4
        max_wallclock_ms: 30000
        max_total_tokens: 8000
      context:
        max_context_tokens: 16000
        persistence: "ephemeral"
    skills:
      - id: "skill:markdown-lint"
        version: "2.1.0"
        source: "registry"
    mcp_tools: []
    tdf_attribute_release_policy:
      attributes: ["https://attr.arkavo.com/clearance/public"]
      rule: "allOf"
    handoffs: []
coordination:
  topology: "hub-spoke"
  protocol: "a2a-jsonrpc-2.0"
  routing:
    strategy: "static"
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  data_classifications: ["public"]
  network:
    egress_allowed: false
    egress_allowlist: []
evaluation:
  rubric:
    dimensions:
      - name: "accuracy"
        weight: 1.0
        threshold: 0.8
  critic_role: "linter"
completion:
  rules: ["all deliverables present"]
  on_failure: "abort"
  max_retries: 0
provenance:
  signatures:
    - signer_did: "did:web:arkavo.net"
      algorithm: "ed25519"
      signature: "-E3rwWzQ_pl92kJP9ZTnQv1fEohUfmhU2d4SE1bJrMXsjli6z6gNs4ZaMAtL0X2qxRDKSgJWZEgNen9ikweYRw"
"#;

#[spec("SK-001")]
#[spec("SK-003")]
#[test]
fn spec_example_parses_validates_and_hashes() {
    let manifest = parse_yaml(SPEC_EXAMPLE_YAML).expect("spec §12.1 example must parse + validate");
    assert_eq!(manifest.kit.name, "Markdown Linter");
    assert_eq!(manifest.roles.len(), 1);
    assert_eq!(manifest.roles[0].role_type, "specialist");

    let kit_id = kit_id_for(&manifest).expect("hash computation should succeed");
    assert!(kit_id.starts_with("blake3:"));
    assert_eq!(kit_id.len(), "blake3:".len() + 43);

    // Two computations on the same input must agree (determinism).
    let kit_id_again = kit_id_for(&manifest).unwrap();
    assert_eq!(kit_id, kit_id_again);
}

#[spec("SK-001")]
#[test]
fn spec_example_with_two_roles_parses_and_validates() {
    let yaml = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "Two-Role Kit"
  version: "1.0.0"
  authors:
    - did: "did:web:arkavo.net"
      name: "Arkavo"
  created: "2026-04-29T12:00:00Z"
  expires: "2026-07-28T12:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "Exercise cross-block validation"
  success_criteria: ["done"]
inputs: []
deliverables: [{name: "out", type: "json"}]
roles:
  - id: "worker"
    role_type: "specialist"
    agent_provisioning:
      model: {family: "qwen3", size: "3B"}
      budget: {max_total_tokens: 4000}
    skills: []
    mcp_tools: []
    handoffs: []
  - id: "critic"
    role_type: "critic"
    agent_provisioning:
      model: {family: "qwen3", size: "3B"}
      budget: {max_total_tokens: 4000}
    skills: []
    mcp_tools: []
    handoffs: []
coordination:
  topology: "hub-spoke"
  routing: {strategy: "static"}
constraints:
  global_budget: {max_wallclock_seconds: 60, max_total_tokens: 8000, max_cost_usd: 0.01}
  data_classifications: ["public"]
  network: {egress_allowed: false, egress_allowlist: []}
evaluation:
  rubric:
    dimensions:
      - name: "quality"
        weight: 1.0
        threshold: 0.8
  critic_role: "critic"
completion:
  rules: ["all deliverables present"]
  on_failure: "abort"
  max_retries: 0
provenance:
  signatures: [{signer_did: "did:web:example.com", algorithm: "ed25519", signature: "AAA"}]
"#;

    let manifest = parse_yaml(yaml).expect("two-role manifest must parse + validate");
    assert_eq!(manifest.roles.len(), 2);
    assert_eq!(manifest.roles[0].id, "worker");
    assert_eq!(manifest.roles[1].id, "critic");
    assert_eq!(manifest.evaluation.as_ref().unwrap().critic_role, "critic");
}
