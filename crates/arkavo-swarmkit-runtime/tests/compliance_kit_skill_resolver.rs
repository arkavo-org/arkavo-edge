//! Closeout test: launch the compliance-kit example end-to-end with
//! skill resolution enabled, asserting every role's skill resolves with
//! verified=true.

use std::sync::Arc;

use arkavo_swarmkit::parse_yaml;
use arkavo_swarmkit_runtime::{
    LaunchOptions, MockPublicKeyResolver, ResolverConfig, SwarmFlight, VerifyMode,
};
use ed25519_dalek::SigningKey;

#[tokio::test]
async fn compliance_kit_resolves_all_role_skills() {
    let yaml = include_str!("../../../examples/compliance-kit/compliance-kit.swarmkit.yaml");
    let manifest = parse_yaml(yaml).expect("parse compliance-kit");

    let dev_key = SigningKey::from_bytes(&[7u8; 32]);
    let mock = MockPublicKeyResolver::new().with_key("did:web:arkavo.com", dev_key.verifying_key());

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
    .expect("launch compliance-kit with resolver");

    for role_id in ["pii_classifier", "policy_enforcer", "auditor"] {
        let role = flight.role(role_id).expect("role exists");
        assert_eq!(
            role.resolved_skills().len(),
            1,
            "role {role_id} should have one resolved skill"
        );
        assert!(
            role.resolved_skills()[0].verified,
            "role {role_id}'s skill should be verified"
        );
    }
}
