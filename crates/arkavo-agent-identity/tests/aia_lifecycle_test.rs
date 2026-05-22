//! End-to-end Agent Identity Authority lifecycle.
//!
//! Walks the ecosystem the AIA role defines: a Root Identity Agent anchors
//! trust, a Swarm Identity Agent issues worker credentials, a verifier (an
//! orchestrator) resolves the trust chain and validates a credential, and the
//! AIA rotates and revokes keys.

use arkavo_agent_identity::{
    AttestationEvidence, IdentityAuthority, IssueRequest, ORCHESTRATOR_ISSUED_ATTRIBUTES, Runtime,
    TrustLevel,
};
use arkavo_attestation::{ModelEvidence, ModelFormat, ModelRuntime};
use arkavo_crypto::AgentKeypair;
use chrono::{Duration, Utc};

fn model_evidence() -> ModelEvidence {
    ModelEvidence {
        model_id: "gemma-4".into(),
        weights_digest: "blake3:0000000000000000000000000000000000000000000000000000000000000000"
            .into(),
        weights_size: 4096,
        format: ModelFormat::Gguf,
        runtime: ModelRuntime::LlamaCpp,
        runtime_version: Some("llama.cpp-b4567".into()),
        architecture: Some("gemma".into()),
        quantization: Some("Q4_K_M".into()),
        gguf_version: Some(3),
        tensor_count: Some(291),
        timestamp: Utc::now().timestamp(),
    }
}

#[test]
fn test_end_to_end_identity_authority_lifecycle() {
    println!("\n=== Agent Identity Authority Lifecycle ===\n");

    println!("Step 1: Stand up the Root Identity Agent (trust anchor)");
    let root = IdentityAuthority::new_root("agent:root-authority");
    let trusted_root = root.verifying_key();
    println!("✓ Root anchored: {}", root.authority_id());

    println!("\nStep 2: Root endorses a Swarm Identity Agent");
    let swarm_keypair = AgentKeypair::generate();
    let endorsement = root
        .endorse_subordinate(
            "agent:swarm-authority",
            &swarm_keypair.public_key(),
            Duration::days(30),
        )
        .expect("endorsement");
    let swarm =
        IdentityAuthority::new_subordinate("agent:swarm-authority", swarm_keypair, endorsement);
    println!("✓ Swarm AIA endorsed by root");

    println!("\nStep 3: A worker generates its own keypair");
    let worker = AgentKeypair::generate();
    println!("✓ Worker holds its private key; only the public key is shared");

    println!("\nStep 4: Swarm AIA issues an attested identity credential");
    let credential = swarm
        .issue(IssueRequest {
            subject: "agent:worker-17".into(),
            agent_type: "coding".into(),
            agent_public_key: worker.public_key(),
            runtime: Runtime::Local,
            capabilities: vec!["a2a".into(), "mcp".into()],
            owner: Some("user:paul".into()),
            organization: Some("arkavo".into()),
            validity: Duration::days(10),
            attestation: Some(AttestationEvidence {
                model: Some(model_evidence()),
                platform: None,
            }),
        })
        .expect("issue");
    assert_eq!(credential.document.trust_level, TrustLevel::Attested);
    assert_eq!(credential.document.model.as_deref(), Some("gemma-4"));
    println!(
        "✓ Credential issued: sub={} trust_level={} model={:?}",
        credential.document.sub,
        credential.document.trust_level.as_str(),
        credential.document.model,
    );

    println!("\nStep 5: An orchestrator resolves the trust chain and verifies");
    let swarm_key = swarm
        .trust_chain()
        .resolve(&trusted_root, Utc::now())
        .expect("trust chain resolves to the root");
    let document = credential
        .verify(&swarm_key, Utc::now())
        .expect("credential verifies under the resolved key");
    println!("✓ Trust chain Root -> Swarm -> credential verified");

    println!("\nStep 6: Identity yields only agent.identity.* OpenTDF attributes");
    let attributes = document.tdf_attributes();
    for attr in &attributes {
        println!("  {} = {}", attr.name, attr.value);
        assert!(attr.name.starts_with("agent.identity."));
    }
    for forbidden in ORCHESTRATOR_ISSUED_ATTRIBUTES {
        assert!(
            !attributes.iter().any(|a| a.name == forbidden),
            "AIA must not originate orchestrator-issued attribute {forbidden}"
        );
    }
    println!("✓ Authorization attributes remain orchestrator-issued");

    println!("\nStep 7: Rotate the worker's key");
    let rotated_key = AgentKeypair::generate();
    let rotated = swarm
        .rotate(&credential, &rotated_key.public_key(), Duration::days(10))
        .expect("rotate");
    assert_eq!(rotated.document.sub, credential.document.sub);
    assert_ne!(rotated.document.public_key, credential.document.public_key);
    rotated
        .verify(&swarm_key, Utc::now())
        .expect("rotated credential verifies");
    println!("✓ Worker re-keyed; identity claims preserved");

    println!("\nStep 8: Revoke the worker and publish the revocation list");
    let mut swarm = swarm;
    swarm.revoke("agent:worker-17", Some("key compromise".into()));
    let revocations = swarm.revocation_list().expect("revocation list");
    revocations
        .verify(&swarm_key)
        .expect("revocation list verifies");
    assert!(revocations.is_revoked("agent:worker-17"));
    println!("✓ Revocation list published and signed by the Swarm AIA");

    println!("\n=== Lifecycle Complete ===\n");
}

#[test]
fn test_credential_from_an_unendorsed_authority_does_not_chain_to_the_root() {
    let root = IdentityAuthority::new_root("agent:root-authority");
    let rogue = IdentityAuthority::new_root("agent:rogue-authority");
    let worker = AgentKeypair::generate();

    let credential = rogue
        .issue(IssueRequest {
            subject: "agent:worker-99".into(),
            agent_type: "coding".into(),
            agent_public_key: worker.public_key(),
            runtime: Runtime::Local,
            capabilities: vec![],
            owner: None,
            organization: None,
            validity: Duration::days(1),
            attestation: None,
        })
        .expect("issue");

    // The rogue AIA can sign its own credential, but its trust chain is not
    // anchored to the root the orchestrator trusts.
    assert!(
        credential
            .verify(&rogue.verifying_key(), Utc::now())
            .is_ok()
    );
    assert!(
        rogue
            .trust_chain()
            .resolve(&root.verifying_key(), Utc::now())
            .is_err()
    );
}
