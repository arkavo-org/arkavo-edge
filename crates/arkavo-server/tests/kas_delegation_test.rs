//! Integration tests for KAS-gated delegation via AGENTS.md trusted roots
//!
//! Regression coverage for KAS trusted roots previously being dropped:
//! the server used to build `KasA2aHandler::new(vec![], ...)`, so every
//! `kas.rewrap` failed with `NoTrustedRoot`. These tests drive the full
//! path: AGENTS.md YAML -> `KasYamlConfig.trusted_roots` ->
//! `trusted_roots_from_config` -> `KasA2aHandler::handle_rewrap`.

#![cfg(feature = "kas")]

use arkavo_crypto::{AgentKeypair, KasEcKeypair};
use arkavo_server::server::handlers::kas::trusted_roots_from_config;
use arkavo_tdf::{
    Attribute, DelegationError, DelegationToken, KasA2aConfig, KasA2aHandler, KasError, KasKeypair,
    KasRewrapRequest, Policy, PolicyBinding,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{Duration, Utc};

const ROLE_ADMIN_FQN: &str = "https://arkavo.net/attr/role/value/admin";

struct ChainFixture {
    root_did: String,
    caller_did: String,
    delegation_token: String,
}

/// Build a two-level delegation chain (root -> intermediate -> caller)
/// signed with real Ed25519 keypairs.
fn make_chain() -> ChainFixture {
    let root_key = AgentKeypair::generate();
    let root_did = root_key.public_key().to_did_key();
    let intermediate_key = AgentKeypair::generate();
    let intermediate_did = intermediate_key.public_key().to_did_key();
    let caller_key = AgentKeypair::generate();
    let caller_did = caller_key.public_key().to_did_key();

    let mut parent = DelegationToken {
        issuer_did: root_did.clone(),
        subject_did: intermediate_did.clone(),
        entitlements: vec![ROLE_ADMIN_FQN.to_string()],
        expires_at: Utc::now() + Duration::hours(1),
        signature: String::new(),
        parent: None,
    };
    parent.signature = BASE64.encode(root_key.sign(&parent.payload_bytes().unwrap()));

    let mut leaf = DelegationToken {
        issuer_did: intermediate_did,
        subject_did: caller_did.clone(),
        entitlements: vec![ROLE_ADMIN_FQN.to_string()],
        expires_at: Utc::now() + Duration::hours(1),
        signature: String::new(),
        parent: Some(Box::new(parent)),
    };
    leaf.signature = BASE64.encode(intermediate_key.sign(&leaf.payload_bytes().unwrap()));

    ChainFixture {
        root_did,
        caller_did,
        delegation_token: leaf.to_json().unwrap(),
    }
}

/// Load an AGENTS.md containing a `kas:` block with the given trusted root
/// DIDs and return the parsed KAS config.
fn load_kas_config(trusted_root_dids: &[&str]) -> arkavo_router::KasYamlConfig {
    let mut roots_yaml = String::new();
    for did in trusted_root_dids {
        roots_yaml.push_str(&format!(
            "    - did: \"{did}\"\n      name: \"Test Root\"\n"
        ));
    }
    let content = format!(
        "---\nname: kas-agent\nkas:\n  enabled: true\n  key_id: test-key\n  algorithm: ec:secp256r1\n  trusted_roots:\n{roots_yaml}---\n"
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("AGENTS.md");
    std::fs::write(&path, content).unwrap();

    let agent_config = arkavo_router::preflight::load_agent_config_from_agents_md(&path).unwrap();
    agent_config.kas.expect("kas config should parse")
}

fn make_rewrap_request(delegation_token: String) -> KasRewrapRequest {
    // NanoTDF header stub: 3-byte magic + 33-byte compressed ephemeral key.
    // The handler only needs a parseable ephemeral public key at offset 3.
    let ephemeral = KasEcKeypair::generate();
    let mut header = b"L1L".to_vec();
    header.extend_from_slice(&ephemeral.public_key_sec1_compressed());

    let policy = Policy {
        id: Some("test-policy".to_string()),
        attributes: vec![Attribute::new("https://arkavo.net/attr/role", &["admin"])],
        dissemination: vec![],
    };
    let policy_json = serde_json::to_string(&policy).unwrap();

    let client_key = KasEcKeypair::generate();

    KasRewrapRequest {
        wrapped_key: BASE64.encode(header),
        policy_binding: PolicyBinding::new("test-binding-hash"),
        policy: BASE64.encode(policy_json.as_bytes()),
        delegation_token,
        client_public_key: client_key.public_key_base64(),
    }
}

#[tokio::test]
async fn rewrap_succeeds_when_chain_terminates_at_configured_root() {
    let chain = make_chain();
    let kas_config = load_kas_config(&[&chain.root_did]);
    let trusted_roots = trusted_roots_from_config(&kas_config);
    assert_eq!(trusted_roots.len(), 1);
    assert_eq!(trusted_roots[0].did, chain.root_did);

    let mut handler = KasA2aHandler::new(trusted_roots, KasA2aConfig::default());
    handler.set_keypair(KasKeypair::generate());

    let request = make_rewrap_request(chain.delegation_token);
    let response = handler.handle_rewrap(request, &chain.caller_did).await;

    assert!(
        response.is_ok(),
        "rewrap should succeed: {:?}",
        response.err()
    );
    assert!(!response.unwrap().entity_wrapped_key.is_empty());
}

#[tokio::test]
async fn rewrap_denied_when_chain_terminates_at_unknown_root() {
    let chain = make_chain();
    // Configure a root that is not the chain's issuer.
    let unknown_root = AgentKeypair::generate().public_key().to_did_key();
    let kas_config = load_kas_config(&[&unknown_root]);
    let trusted_roots = trusted_roots_from_config(&kas_config);

    let mut handler = KasA2aHandler::new(trusted_roots, KasA2aConfig::default());
    handler.set_keypair(KasKeypair::generate());

    let request = make_rewrap_request(chain.delegation_token);
    let result = handler.handle_rewrap(request, &chain.caller_did).await;

    assert!(matches!(
        result,
        Err(KasError::Delegation(DelegationError::NoTrustedRoot))
    ));
}

#[tokio::test]
async fn rewrap_denied_when_no_roots_configured() {
    // Regression guard for the original deny-all behavior: without
    // trusted_roots in AGENTS.md the handler must keep denying.
    let chain = make_chain();
    let kas_config = load_kas_config(&[]);
    let trusted_roots = trusted_roots_from_config(&kas_config);
    assert!(trusted_roots.is_empty());

    let mut handler = KasA2aHandler::new(trusted_roots, KasA2aConfig::default());
    handler.set_keypair(KasKeypair::generate());

    let request = make_rewrap_request(chain.delegation_token);
    let result = handler.handle_rewrap(request, &chain.caller_did).await;

    assert!(matches!(
        result,
        Err(KasError::Delegation(DelegationError::NoTrustedRoot))
    ));
}
