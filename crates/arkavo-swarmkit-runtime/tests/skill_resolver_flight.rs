//! Integration tests for SwarmFlight::launch with skill resolution.

use std::sync::Arc;

use arkavo_swarmkit::{Manifest, SkillContent, parse_yaml};
use arkavo_swarmkit_runtime::{
    LaunchError, LaunchOptions, MockPublicKeyResolver, ResolveError, ResolverConfig, SwarmFlight,
    VerifyMode, sign_skill_content,
};
use ed25519_dalek::SigningKey;

fn deterministic_signer() -> (SigningKey, &'static str) {
    (
        SigningKey::from_bytes(&[42u8; 32]),
        "did:web:test.arkavo.com",
    )
}

fn kit_with_two_inline_skills(signed_payloads: [(serde_json::Value, String, String); 2]) -> String {
    let [(p0, sig0, by0), (p1, sig1, by1)] = signed_payloads;
    format!(
        r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "skills-flight"
  version: "0.1.0"
  authors: [{{did: "did:web:example.com"}}]
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "two inline skills on one role"
  success_criteria: ["resolved"]
inputs: []
deliverables: [{{name: "out", type: "json"}}]
roles:
  - id: "alpha"
    role_type: "specialist"
    agent_provisioning: {{}}
    skills:
      - id: "skill:s0"
        version: "0.1.0"
        source: "inline"
        payload: {p0}
        signature: "{sig0}"
        signed_by: "{by0}"
      - id: "skill:s1"
        version: "0.1.0"
        source: "inline"
        payload: {p1}
        signature: "{sig1}"
        signed_by: "{by1}"
coordination:
  topology: "hub-spoke"
  routing: {{strategy: "static"}}
constraints:
  global_budget: {{max_wallclock_seconds: 60, max_total_tokens: 8000, max_cost_usd: 0.10}}
  data_classifications: ["public"]
  network: {{egress_allowed: false, egress_allowlist: []}}
completion:
  rules: ["all deliverables present"]
  on_failure: "abort"
  max_retries: 0
provenance:
  signatures: [{{signer_did: "did:web:example.com", algorithm: "ed25519", signature: "AAA"}}]
"#
    )
}

#[tokio::test]
async fn t11_flight_launch_with_resolved_skills() {
    let (key, did) = deterministic_signer();
    let make_content = |name: &str| SkillContent {
        name: name.into(),
        description: format!("desc {name}"),
        instructions: format!("inst {name}"),
        resources: vec![],
    };
    let s0 = make_content("s0");
    let s1 = make_content("s1");
    let sig0 = sign_skill_content(&s0, did, &key);
    let sig1 = sign_skill_content(&s1, did, &key);
    let yaml = kit_with_two_inline_skills([
        (
            serde_json::to_value(&s0).unwrap(),
            sig0.signature_b64url,
            sig0.signed_by,
        ),
        (
            serde_json::to_value(&s1).unwrap(),
            sig1.signature_b64url,
            sig1.signed_by,
        ),
    ]);
    let manifest: Manifest = parse_yaml(&yaml).unwrap();

    let mock = MockPublicKeyResolver::new().with_key(did, key.verifying_key());
    let cfg = ResolverConfig {
        registry_cache: std::env::temp_dir(),
        verify: VerifyMode::Required,
        public_key_resolver: Arc::new(mock),
    };
    let flight = SwarmFlight::launch(
        &manifest,
        LaunchOptions {
            resolver_config: Some(cfg),
            ..LaunchOptions::default()
        },
    )
    .unwrap();

    let role = flight.role("alpha").expect("alpha role");
    assert_eq!(role.resolved_skills().len(), 2);
    assert!(role.resolved_skills().iter().all(|r| r.verified));
}

#[tokio::test]
async fn t12_flight_launch_aborts_on_unresolvable_skill() {
    let yaml = r#"
spec_version: "1.0.0"
kit: {id: "", name: "x", version: "0.1.0", authors: [{did: "did:web:e.com"}], created: "2026-04-29T00:00:00Z", expires: "2026-05-29T00:00:00Z", nonce: "thz1Cz8aWOUURbyQQfvA0Q"}
objective: {goal: "x", success_criteria: ["y"]}
inputs: []
deliverables: [{name: "out", type: "json"}]
roles:
  - id: "alpha"
    role_type: "specialist"
    agent_provisioning: {}
    skills:
      - id: "blake3:nonexistent"
        version: "0.1.0"
        source: "registry"
coordination: {topology: "hub-spoke", routing: {strategy: "static"}}
constraints:
  global_budget: {max_wallclock_seconds: 60, max_total_tokens: 8000, max_cost_usd: 0.10}
  data_classifications: ["public"]
  network: {egress_allowed: false, egress_allowlist: []}
completion: {rules: ["x"], on_failure: "abort", max_retries: 0}
provenance: {signatures: [{signer_did: "did:web:e.com", algorithm: "ed25519", signature: "AAA"}]}
"#;
    let manifest: Manifest = parse_yaml(yaml).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cfg = ResolverConfig {
        registry_cache: dir.path().to_path_buf(),
        verify: VerifyMode::Optional,
        public_key_resolver: Arc::new(MockPublicKeyResolver::new()),
    };
    let err = SwarmFlight::launch(
        &manifest,
        LaunchOptions {
            resolver_config: Some(cfg),
            ..LaunchOptions::default()
        },
    )
    .unwrap_err();
    match err {
        LaunchError::SkillResolution { role, source } => {
            assert_eq!(role, "alpha");
            assert!(matches!(source, ResolveError::RegistryMiss { .. }));
        }
        other => panic!("expected SkillResolution, got {other:?}"),
    }
}
