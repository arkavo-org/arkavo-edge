//! Closeout test for Phase 2 Skill resolver: launch the campaign-kit
//! example end-to-end with skill resolution enabled, asserting every
//! role's skill resolves and verifies.

use std::sync::Arc;

use arkavo_swarmkit::parse_yaml;
use arkavo_swarmkit_runtime::{
    LaunchOptions, MockPublicKeyResolver, ResolverConfig, SwarmFlight, VerifyMode,
};
use ed25519_dalek::SigningKey;

const CAMPAIGN_KIT: &str =
    include_str!("../../../examples/campaign-kit/campaign-kit.swarmkit.yaml");

#[tokio::test]
async fn campaign_kit_resolves_all_role_skills() {
    let manifest = parse_yaml(CAMPAIGN_KIT).expect("parse campaign-kit");

    // The campaign-kit YAML is signed with the deterministic dev key
    // ([7u8; 32]) from sign_campaign_skills. We supply the matching
    // pubkey via a mock resolver so the test runs offline.
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
    .expect("launch campaign-kit with resolver");

    for role_id in ["analyst", "copy", "critic"] {
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
