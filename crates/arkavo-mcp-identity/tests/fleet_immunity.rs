//! Integration test simulating the fleet-immunity example with MCP-I proofs.
//!
//! Scenario: Two rovers (Alpha and Beta) exchange tool results with signed
//! identity proofs. Alpha queries a sector, signs the response, and Beta
//! verifies the proof — demonstrating end-to-end MCP-I L1 identity flow.

use arkavo_mcp_identity::{McpIdentitySession, register_identity_tool};
use serde_json::json;

/// Simulated get_sector tool result (matches fleet-immunity example format)
fn get_sector_result(id: u8, name: &str, hazard: Option<(&str, f32)>) -> serde_json::Value {
    json!({
        "id": id,
        "name": name,
        "hazard": hazard.map(|(h, t)| json!({"type": h, "traction": t}))
    })
}

#[test]
fn rover_signs_sector_query_and_peer_verifies() {
    // Rover Alpha: queries sector 4, signs the result
    let alpha = McpIdentitySession::ephemeral();
    let result = get_sector_result(4, "Cold Storage", Some(("black_ice", 0.1)));
    let meta = alpha.sign_response("get_sector", &result);

    // Verify proof structure
    let proof = &meta["identityProof"];
    assert_eq!(proof["type"], "detached-jws-ed25519");
    assert!(
        proof["jws"].as_str().unwrap().contains(".."),
        "expected detached JWS format"
    );
    assert!(proof["did"].as_str().unwrap().starts_with("did:key:z"));
    assert!(proof["timestamp"].is_string());
    assert!(proof["nonce"].is_string());

    // Rover Beta: receives the result + proof, verifies Alpha's identity
    let mut beta = McpIdentitySession::ephemeral();
    let verified_did = beta
        .verify_peer_proof(&meta, "get_sector", &result)
        .expect("proof verification should succeed");

    assert_eq!(verified_did, alpha.did());

    // Beta now has Alpha in its reputation tracker
    let rep = beta.reputation(alpha.did()).unwrap();
    assert_eq!(rep.successful_verifications, 1);
    assert_eq!(rep.failed_verifications, 0);
}

#[test]
fn tampered_hazard_data_is_detected() {
    // Alpha signs a safe sector result
    let alpha = McpIdentitySession::ephemeral();
    let original = get_sector_result(4, "Cold Storage", None);
    let meta = alpha.sign_response("get_sector", &original);

    // Attacker injects a fake hazard into the result
    let tampered = get_sector_result(4, "Cold Storage", Some(("black_ice", 0.1)));

    // Beta detects the tampering
    let mut beta = McpIdentitySession::ephemeral();
    let err = beta.verify_peer_proof(&meta, "get_sector", &tampered);
    assert!(err.is_err(), "tampered result should fail verification");

    // Failure is tracked in reputation
    let rep = beta.reputation(alpha.did()).unwrap();
    assert_eq!(rep.successful_verifications, 0);
    assert_eq!(rep.failed_verifications, 1);
}

#[test]
fn tool_name_mismatch_is_detected() {
    // Alpha signs a get_sector result
    let alpha = McpIdentitySession::ephemeral();
    let result = get_sector_result(1, "Loading Dock", None);
    let meta = alpha.sign_response("get_sector", &result);

    // If someone claims this was an inject_hazard response, verification fails
    let mut beta = McpIdentitySession::ephemeral();
    let err = beta.verify_peer_proof(&meta, "inject_hazard", &result);
    assert!(err.is_err(), "wrong tool name should fail verification");
}

#[test]
fn multiple_tool_calls_build_reputation() {
    let alpha = McpIdentitySession::ephemeral();
    let mut beta = McpIdentitySession::ephemeral();

    // Alpha makes 3 sector queries, Beta verifies each
    for sector_id in 1..=3 {
        let names = ["Loading Dock", "Main Aisle", "Storage A"];
        let result = get_sector_result(sector_id, names[(sector_id - 1) as usize], None);
        let meta = alpha.sign_response("get_sector", &result);
        beta.verify_peer_proof(&meta, "get_sector", &result)
            .unwrap();
    }

    let rep = beta.reputation(alpha.did()).unwrap();
    assert_eq!(rep.successful_verifications, 3);
    assert_eq!(rep.failed_verifications, 0);
    assert!(rep.first_seen <= rep.last_seen);
}

#[test]
fn two_rovers_exchange_proofs_bidirectionally() {
    let mut alpha = McpIdentitySession::ephemeral();
    let mut beta = McpIdentitySession::ephemeral();

    // Alpha → Beta: sector query
    let r1 = get_sector_result(1, "Loading Dock", None);
    let m1 = alpha.sign_response("get_sector", &r1);
    let did = beta.verify_peer_proof(&m1, "get_sector", &r1).unwrap();
    assert_eq!(did, alpha.did());

    // Beta → Alpha: hazard injection confirmation
    let r2 = json!({"success": true, "sector": 4, "hazard_type": "black_ice"});
    let m2 = beta.sign_response("inject_hazard", &r2);
    let did = alpha.verify_peer_proof(&m2, "inject_hazard", &r2).unwrap();
    assert_eq!(did, beta.did());

    // Both have each other tracked
    assert_eq!(
        alpha
            .reputation(beta.did())
            .unwrap()
            .successful_verifications,
        1
    );
    assert_eq!(
        beta.reputation(alpha.did())
            .unwrap()
            .successful_verifications,
        1
    );
}

#[tokio::test]
async fn mcpi_protocol_tool_identity_disclosure() {
    use arkavo_mcp_tools::ToolRegistry;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    let session = Arc::new(RwLock::new(McpIdentitySession::ephemeral()));
    let storage: Arc<arkavo_memory::MemoryStorage> = Arc::new(
        arkavo_memory::MemoryStorage::new()
            .await
            .expect("memory storage"),
    );
    let mut registry = ToolRegistry::new(storage);
    register_identity_tool(&mut registry, session.clone());

    // Call _mcpi identity action
    let tool = registry
        .get("_mcpi")
        .expect("_mcpi tool should be registered");
    let result = tool.execute(json!({"action": "identity"})).await.unwrap();

    assert_eq!(result["identityLevel"], 1);
    assert_eq!(result["algorithms"][0], "EdDSA");
    assert!(result["did"].as_str().unwrap().starts_with("did:key:z"));
    assert!(result["sessionId"].is_string());

    // Verify a peer DID
    let peer = arkavo_crypto::AgentKeypair::generate();
    let peer_did = peer.public_key().to_did_key();
    let result = tool
        .execute(json!({"action": "verify", "did": peer_did}))
        .await
        .unwrap();
    assert_eq!(result["verified"], true);

    // Check session state
    let result = tool.execute(json!({"action": "session"})).await.unwrap();
    assert_eq!(result["peerDid"], peer_did);
    assert_eq!(result["verified"], true);
}
