//! Regenerate the published test vectors in `tests/vectors/`.
//!
//! Run from the crate root: `cargo run -p arkavo-permit --example generate_vectors`
//!
//! The vectors are fully deterministic: fixed secret keys, fixed claims, and
//! fixed timestamps, so re-running this example must produce byte-identical
//! files. `tests/vectors_test.rs` verifies the committed files.

use std::path::PathBuf;

use arkavo_crypto::{AgentKeypair, P256SigningKeypair};
use arkavo_permit::{Budget, HashAlgorithm, PermitClaims, PermitSigner, argument_hash, mint};
use serde_json::{Value, json};

const IAT: i64 = 1_700_000_000;
const EXP: i64 = 1_700_000_300;

fn base_claims(tool_name: &str, arguments: &Value, hash_algorithm: HashAlgorithm) -> PermitClaims {
    PermitClaims {
        issuer: "https://permit-issuer.arkavo.example".to_string(),
        subject: "did:example:human-alice".to_string(),
        agent_workload_id: "spiffe://arkavo-edge/ns/default/sa/edge-agent-001".to_string(),
        policy_bundle_hash: hash_algorithm.digest(b"arkavo-policy-bundle-2026-08"),
        tool_name: tool_name.to_string(),
        argument_hash: argument_hash(arguments, hash_algorithm),
        data_classifications: vec![
            "tdf:classification:confidential".to_string(),
            "tdf:region:us".to_string(),
        ],
        budget: Budget {
            max_invocations: 5,
            token_ceiling: Some(100_000),
            cost_micro_usd: Some(250_000),
        },
        sequence_state_hash: hash_algorithm.digest(b"sequence-state-0"),
        issued_at: IAT,
        not_before: IAT,
        expires_at: EXP,
        parent_permit: None,
    }
}

fn vector_json(
    name: &str,
    signer: &PermitSigner,
    secret_key_hex: &str,
    claims: &PermitClaims,
    arguments: &Value,
    hash_algorithm: HashAlgorithm,
) -> Value {
    let cwt = mint(claims, signer).expect("mint permit");
    let algorithm = match signer {
        PermitSigner::Ed25519(_) => "EdDSA",
        PermitSigner::P256(_) => "ES256",
    };
    json!({
        "name": name,
        "description": "Signed CWT permit test vector for arkavo-permit; regenerate with `cargo run -p arkavo-permit --example generate_vectors`",
        "algorithm": algorithm,
        "hash_algorithm": hash_algorithm.name(),
        "secret_key_hex": secret_key_hex,
        "public_key_hex": hex::encode(signer.public_key().public_key_bytes()),
        "now_for_verification": IAT + 60,
        "cwt_hex": hex::encode(cwt),
        "claims": {
            "iss": claims.issuer,
            "sub": claims.subject,
            "agent_workload_id": claims.agent_workload_id,
            "policy_bundle_hash_hex": hex::encode(&claims.policy_bundle_hash),
            "tool_name": claims.tool_name,
            "arguments": arguments,
            "argument_hash_hex": hex::encode(&claims.argument_hash),
            "data_classifications": claims.data_classifications,
            "budget": {
                "max_invocations": claims.budget.max_invocations,
                "token_ceiling": claims.budget.token_ceiling,
                "cost_micro_usd": claims.budget.cost_micro_usd,
            },
            "sequence_state_hash_hex": hex::encode(&claims.sequence_state_hash),
            "iat": claims.issued_at,
            "nbf": claims.not_before,
            "exp": claims.expires_at,
            "parent_permit_hex": claims.parent_permit.as_ref().map(hex::encode),
        }
    })
}

fn write_vector(dir: &std::path::Path, name: &str, vector: &Value) {
    let path = dir.join(format!("{name}.json"));
    let mut text = serde_json::to_string_pretty(vector).expect("serialize vector");
    text.push('\n');
    std::fs::write(&path, text).expect("write vector file");
    println!("wrote {}", path.display());
}

fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vectors");
    std::fs::create_dir_all(&dir).expect("create vectors dir");

    // Vector 1: Ed25519 (EdDSA) with SHA-256 hashes.
    let ed_secret = [0x11u8; 32];
    let ed_signer =
        PermitSigner::Ed25519(AgentKeypair::from_bytes(&ed_secret).expect("valid ed25519 secret"));
    let read_args = json!({"path": "/tmp/data.csv", "max_bytes": 4096});
    let claims = base_claims("arkavo.fs.read_file", &read_args, HashAlgorithm::Sha256);
    let vector = vector_json(
        "ed25519-sha256",
        &ed_signer,
        &hex::encode(ed_secret),
        &claims,
        &read_args,
        HashAlgorithm::Sha256,
    );
    let parent_cwt_hex = vector["cwt_hex"].as_str().expect("cwt hex").to_string();
    write_vector(&dir, "ed25519-sha256", &vector);

    // Vector 2: P-256 (ES256) with BLAKE3 hashes.
    let p256_secret = [0x22u8; 32];
    let p256_signer = PermitSigner::P256(
        P256SigningKeypair::from_bytes(&p256_secret).expect("valid p256 secret"),
    );
    let exec_args = json!({"argv": ["ls", "-la"], "cwd": "/tmp", "timeout_ms": 5000});
    let claims = base_claims("arkavo.shell.exec", &exec_args, HashAlgorithm::Blake3);
    let vector = vector_json(
        "es256-blake3",
        &p256_signer,
        &hex::encode(p256_secret),
        &claims,
        &exec_args,
        HashAlgorithm::Blake3,
    );
    write_vector(&dir, "es256-blake3", &vector);

    // Vector 3: Ed25519 child permit chained to vector 1 via parent_permit.
    let child_secret = [0x33u8; 32];
    let child_signer = PermitSigner::Ed25519(
        AgentKeypair::from_bytes(&child_secret).expect("valid ed25519 secret"),
    );
    let delegate_args =
        json!({"agent": "spiffe://arkavo-edge/ns/default/sa/edge-agent-002", "task": "summarize"});
    let mut claims = base_claims("arkavo.a2a.delegate", &delegate_args, HashAlgorithm::Sha256);
    claims.parent_permit =
        Some(HashAlgorithm::Sha256.digest(&hex::decode(&parent_cwt_hex).expect("parent cwt hex")));
    let vector = vector_json(
        "ed25519-parent-chain",
        &child_signer,
        &hex::encode(child_secret),
        &claims,
        &delegate_args,
        HashAlgorithm::Sha256,
    );
    write_vector(&dir, "ed25519-parent-chain", &vector);
}
