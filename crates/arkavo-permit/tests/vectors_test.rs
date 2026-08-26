//! Validates the published test vectors in `tests/vectors/` against the
//! implementation. Vectors are regenerated with
//! `cargo run -p arkavo-permit --example generate_vectors`.

use std::path::PathBuf;

use arkavo_permit::{Budget, HashAlgorithm, verify};
use serde_json::Value;

fn load_vector(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vectors")
        .join(format!("{name}.json"));
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn hex_decode(value: &Value, field: &str) -> Vec<u8> {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("{field} must be a string"));
    hex::decode(text).unwrap_or_else(|e| panic!("{field} hex: {e}"))
}

fn check_vector(vector: &Value) {
    let name = vector["name"].as_str().unwrap();
    let hash_algorithm = HashAlgorithm::from_name(vector["hash_algorithm"].as_str().unwrap())
        .expect("known hash algorithm");
    let cwt = hex_decode(&vector["cwt_hex"], "cwt_hex");
    let now = vector["now_for_verification"].as_i64().unwrap();
    let expected = &vector["claims"];

    let permit = verify(&cwt, now).unwrap_or_else(|e| panic!("{name}: verify: {e}"));
    let claims = &permit.claims;

    assert_eq!(
        claims.issuer,
        expected["iss"].as_str().unwrap(),
        "{name} iss"
    );
    assert_eq!(
        claims.subject,
        expected["sub"].as_str().unwrap(),
        "{name} sub"
    );
    assert_eq!(
        claims.agent_workload_id,
        expected["agent_workload_id"].as_str().unwrap(),
        "{name} agent_workload_id"
    );
    assert_eq!(
        claims.policy_bundle_hash,
        hex_decode(
            &expected["policy_bundle_hash_hex"],
            "policy_bundle_hash_hex"
        ),
        "{name} policy_bundle_hash"
    );
    assert_eq!(
        claims.tool_name,
        expected["tool_name"].as_str().unwrap(),
        "{name} tool_name"
    );
    assert_eq!(
        claims.argument_hash,
        hex_decode(&expected["argument_hash_hex"], "argument_hash_hex"),
        "{name} argument_hash"
    );
    let classifications: Vec<String> = expected["data_classifications"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        claims.data_classifications, classifications,
        "{name} data_classifications"
    );
    let budget = &expected["budget"];
    assert_eq!(
        claims.budget,
        Budget {
            max_invocations: budget["max_invocations"].as_u64().unwrap(),
            token_ceiling: budget["token_ceiling"].as_u64(),
            cost_micro_usd: budget["cost_micro_usd"].as_u64(),
        },
        "{name} budget"
    );
    assert_eq!(
        claims.sequence_state_hash,
        hex_decode(
            &expected["sequence_state_hash_hex"],
            "sequence_state_hash_hex"
        ),
        "{name} sequence_state_hash"
    );
    assert_eq!(
        claims.issued_at,
        expected["iat"].as_i64().unwrap(),
        "{name} iat"
    );
    assert_eq!(
        claims.not_before,
        expected["nbf"].as_i64().unwrap(),
        "{name} nbf"
    );
    assert_eq!(
        claims.expires_at,
        expected["exp"].as_i64().unwrap(),
        "{name} exp"
    );
    match &expected["parent_permit_hex"] {
        Value::Null => assert_eq!(claims.parent_permit, None, "{name} parent_permit"),
        value => assert_eq!(
            claims.parent_permit.as_deref(),
            Some(hex_decode(value, "parent_permit_hex").as_slice()),
            "{name} parent_permit"
        ),
    }

    // The confirmation key in the permit must equal the published public key.
    assert_eq!(
        permit.confirmation_key.public_key_bytes(),
        hex_decode(&vector["public_key_hex"], "public_key_hex"),
        "{name} cnf public key"
    );

    // The recorded invocation must match the permit binding.
    claims
        .verify_invocation(
            expected["tool_name"].as_str().unwrap(),
            &expected["arguments"],
            hash_algorithm,
        )
        .unwrap_or_else(|e| panic!("{name}: invocation binding: {e}"));

    // Expired permits must be rejected.
    assert!(
        verify(&cwt, claims.expires_at).is_err(),
        "{name}: expired permit accepted"
    );
}

#[test]
fn vector_ed25519_sha256() {
    check_vector(&load_vector("ed25519-sha256"));
}

#[test]
fn vector_es256_blake3() {
    check_vector(&load_vector("es256-blake3"));
}

#[test]
fn vector_ed25519_parent_chain() {
    let child = load_vector("ed25519-parent-chain");
    check_vector(&child);

    // The child permit's parent_permit claim must hash the parent's CWT.
    let parent = load_vector("ed25519-sha256");
    let parent_cwt = hex_decode(&parent["cwt_hex"], "cwt_hex");
    let expected_hash = HashAlgorithm::Sha256.digest(&parent_cwt);
    assert_eq!(
        child["claims"]["parent_permit_hex"].as_str().unwrap(),
        hex::encode(expected_hash)
    );
}

#[test]
fn tampered_vector_rejected() {
    let vector = load_vector("ed25519-sha256");
    let mut cwt = hex_decode(&vector["cwt_hex"], "cwt_hex");
    let now = vector["now_for_verification"].as_i64().unwrap();
    let last = cwt.len() - 1;
    cwt[last] ^= 0x01;
    assert!(verify(&cwt, now).is_err(), "tampered CWT accepted");
}
