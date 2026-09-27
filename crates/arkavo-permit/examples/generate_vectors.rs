//! Regenerate the published test vectors in `tests/vectors/`.
//!
//! Run from the crate root: `cargo run -p arkavo-permit --example generate_vectors`
//!
//! The vectors are fully deterministic: fixed secret keys, fixed claims, and
//! fixed timestamps, so re-running this example must produce byte-identical
//! files. `tests/vectors_test.rs` verifies the committed files.
//!
//! Each vector carries two keypairs, because a permit has two roles: the
//! issuer signs the CWT and is named by `kid_hex` in the protected header,
//! while the `cnf` claim holds the presenter's public key.

use std::path::PathBuf;

use arkavo_crypto::{AgentKeypair, P256SigningKeypair};
use arkavo_permit::{
    Budget, HashAlgorithm, PermitClaims, PermitSigner, argument_hash, decode, invocation_digest,
    issuer_kid, mint, prove_invocation,
};
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

/// One vector's inputs: the issuing key, the presenter's key, and the claims
/// they are bound to.
struct Vector<'a> {
    name: &'a str,
    issuer: PermitSigner,
    issuer_secret: [u8; 32],
    confirmation: PermitSigner,
    confirmation_secret: [u8; 32],
    claims: PermitClaims,
    arguments: &'a Value,
    hash_algorithm: HashAlgorithm,
}

impl Vector<'_> {
    fn to_json(&self) -> Value {
        let issuer_key = self.issuer.public_key();
        let confirmation_key = self.confirmation.public_key();
        let cwt = mint(&self.claims, &self.issuer, &confirmation_key).expect("mint permit");
        let algorithm = match self.issuer {
            PermitSigner::Ed25519(_) => "EdDSA",
            PermitSigner::P256(_) => "ES256",
        };
        let claims = &self.claims;
        // The proof-of-possession the presenter would send with this exact
        // invocation. It names the permit by `Permit::id`, so the vector
        // pins the digest's wire format and not merely a round trip.
        let permit_id = decode(&cwt).expect("decode minted permit").id;
        let pop_digest = invocation_digest(
            &permit_id,
            &claims.tool_name,
            self.arguments,
            self.hash_algorithm,
        );
        let pop_proof = prove_invocation(
            &self.confirmation,
            &permit_id,
            &claims.tool_name,
            self.arguments,
            self.hash_algorithm,
        );
        json!({
            "name": self.name,
            "description": "Signed CWT permit test vector for arkavo-permit; the issuer signs and `cnf` holds the presenter's key. Regenerate with `cargo run -p arkavo-permit --example generate_vectors`",
            "algorithm": algorithm,
            "hash_algorithm": self.hash_algorithm.name(),
            "issuer_secret_key_hex": hex::encode(self.issuer_secret),
            "issuer_public_key_hex": hex::encode(issuer_key.public_key_bytes()),
            "kid_hex": hex::encode(issuer_kid(&issuer_key)),
            "cnf_secret_key_hex": hex::encode(self.confirmation_secret),
            "cnf_public_key_hex": hex::encode(confirmation_key.public_key_bytes()),
            "now_for_verification": IAT + 60,
            "cwt_hex": hex::encode(cwt),
            "permit_id_hex": hex::encode(permit_id),
            "pop_tool_name": claims.tool_name,
            "pop_arguments_json": serde_json::to_string(self.arguments).expect("arguments json"),
            "pop_digest_hex": hex::encode(pop_digest),
            "pop_proof_hex": hex::encode(pop_proof),
            "claims": {
                "iss": claims.issuer,
                "sub": claims.subject,
                "agent_workload_id": claims.agent_workload_id,
                "policy_bundle_hash_hex": hex::encode(&claims.policy_bundle_hash),
                "tool_name": claims.tool_name,
                "arguments": self.arguments,
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
}

fn ed25519(secret: [u8; 32]) -> PermitSigner {
    PermitSigner::Ed25519(AgentKeypair::from_bytes(&secret).expect("valid ed25519 secret"))
}

fn p256(secret: [u8; 32]) -> PermitSigner {
    PermitSigner::P256(P256SigningKeypair::from_bytes(&secret).expect("valid p256 secret"))
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

    // Vector 1: Ed25519 (EdDSA) issuer with SHA-256 hashes.
    let read_args = json!({"path": "/tmp/data.csv", "max_bytes": 4096});
    let issuer_secret = [0x11u8; 32];
    let confirmation_secret = [0xa1u8; 32];
    let vector = Vector {
        name: "ed25519-sha256",
        issuer: ed25519(issuer_secret),
        issuer_secret,
        confirmation: ed25519(confirmation_secret),
        confirmation_secret,
        claims: base_claims("arkavo.fs.read_file", &read_args, HashAlgorithm::Sha256),
        arguments: &read_args,
        hash_algorithm: HashAlgorithm::Sha256,
    }
    .to_json();
    let parent_cwt_hex = vector["cwt_hex"].as_str().expect("cwt hex").to_string();
    write_vector(&dir, "ed25519-sha256", &vector);

    // Vector 2: P-256 (ES256) issuer and presenter, with BLAKE3 hashes.
    let exec_args = json!({"argv": ["ls", "-la"], "cwd": "/tmp", "timeout_ms": 5000});
    let issuer_secret = [0x22u8; 32];
    let confirmation_secret = [0xa2u8; 32];
    let vector = Vector {
        name: "es256-blake3",
        issuer: p256(issuer_secret),
        issuer_secret,
        confirmation: p256(confirmation_secret),
        confirmation_secret,
        claims: base_claims("arkavo.shell.exec", &exec_args, HashAlgorithm::Blake3),
        arguments: &exec_args,
        hash_algorithm: HashAlgorithm::Blake3,
    }
    .to_json();
    write_vector(&dir, "es256-blake3", &vector);

    // Vector 3: Ed25519 child permit chained to vector 1 via parent_permit.
    let delegate_args =
        json!({"agent": "spiffe://arkavo-edge/ns/default/sa/edge-agent-002", "task": "summarize"});
    let mut claims = base_claims("arkavo.a2a.delegate", &delegate_args, HashAlgorithm::Sha256);
    claims.parent_permit =
        Some(HashAlgorithm::Sha256.digest(&hex::decode(&parent_cwt_hex).expect("parent cwt hex")));
    let issuer_secret = [0x33u8; 32];
    let confirmation_secret = [0xa3u8; 32];
    let vector = Vector {
        name: "ed25519-parent-chain",
        issuer: ed25519(issuer_secret),
        issuer_secret,
        confirmation: ed25519(confirmation_secret),
        confirmation_secret,
        claims,
        arguments: &delegate_args,
        hash_algorithm: HashAlgorithm::Sha256,
    }
    .to_json();
    write_vector(&dir, "ed25519-parent-chain", &vector);
}
