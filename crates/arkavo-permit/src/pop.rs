//! Proof-of-possession over one invocation. The permit binds tool and
//! argument hash; the proof shows the caller holds the `cnf` private key
//! for exactly this permit and exactly these arguments, so a captured
//! permit is useless to anyone else and to the same agent with other args.

use crate::canonical::argument_hash;
use crate::error::PermitError;
use crate::hash::HashAlgorithm;
use crate::keys::PermitSigner;
use crate::permit::Permit;
use serde_json::Value;

const DOMAIN: &[u8] = b"arkavo-permit-pop/v1";

pub fn invocation_digest(
    permit_cwt: &[u8],
    tool_name: &str,
    arguments: &Value,
    algorithm: HashAlgorithm,
) -> Vec<u8> {
    let mut input = Vec::with_capacity(DOMAIN.len() + 32 + 8 + tool_name.len() + 32);
    input.extend_from_slice(DOMAIN);
    input.extend_from_slice(&algorithm.digest(permit_cwt));
    input.extend_from_slice(&(tool_name.len() as u64).to_be_bytes());
    input.extend_from_slice(tool_name.as_bytes());
    input.extend_from_slice(&argument_hash(arguments, algorithm));
    algorithm.digest(&input)
}

pub fn prove_invocation(
    signer: &PermitSigner,
    permit_cwt: &[u8],
    tool_name: &str,
    arguments: &Value,
    algorithm: HashAlgorithm,
) -> Vec<u8> {
    signer.sign(&invocation_digest(
        permit_cwt, tool_name, arguments, algorithm,
    ))
}

pub fn verify_invocation_proof(
    permit: &Permit,
    permit_cwt: &[u8],
    tool_name: &str,
    arguments: &Value,
    proof: &[u8],
    algorithm: HashAlgorithm,
) -> Result<(), PermitError> {
    let digest = invocation_digest(permit_cwt, tool_name, arguments, algorithm);
    permit
        .confirmation_key
        .verify(permit.confirmation_key.algorithm(), &digest, proof)
        .map_err(|_| PermitError::InvalidProof)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::{Budget, PermitClaims};
    use crate::keys::PermitSigner;
    use crate::{argument_hash, mint, verify};
    use arkavo_crypto::AgentKeypair;
    use serde_json::json;

    const NOW: i64 = 1_700_000_060;

    fn permit_for(signer: &PermitSigner, tool: &str, args: &serde_json::Value) -> Vec<u8> {
        let claims = PermitClaims {
            issuer: "edge".into(),
            subject: "agent-1".into(),
            expires_at: NOW + 300,
            not_before: NOW - 60,
            issued_at: NOW - 60,
            agent_workload_id: "wl-1".into(),
            policy_bundle_hash: vec![7; 32],
            tool_name: tool.into(),
            argument_hash: argument_hash(args, HashAlgorithm::Sha256),
            data_classifications: vec![],
            budget: Budget {
                max_invocations: 3,
                token_ceiling: None,
                cost_micro_usd: None,
            },
            sequence_state_hash: vec![9; 32],
            parent_permit: None,
        };
        mint(&claims, signer).unwrap()
    }

    #[test]
    fn proof_from_the_cnf_key_verifies() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 42});
        let cwt = permit_for(&signer, "github.merge_pr", &args);
        let proof = prove_invocation(
            &signer,
            &cwt,
            "github.merge_pr",
            &args,
            HashAlgorithm::Sha256,
        );
        let permit = verify(&cwt, NOW).unwrap();
        verify_invocation_proof(
            &permit,
            &cwt,
            "github.merge_pr",
            &args,
            &proof,
            HashAlgorithm::Sha256,
        )
        .unwrap();
    }

    #[test]
    fn replay_with_different_arguments_is_rejected() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 42});
        let cwt = permit_for(&signer, "github.merge_pr", &args);
        let proof = prove_invocation(
            &signer,
            &cwt,
            "github.merge_pr",
            &args,
            HashAlgorithm::Sha256,
        );
        let permit = verify(&cwt, NOW).unwrap();
        let other = json!({"pr": 43});
        assert!(matches!(
            verify_invocation_proof(
                &permit,
                &cwt,
                "github.merge_pr",
                &other,
                &proof,
                HashAlgorithm::Sha256
            ),
            Err(PermitError::InvalidProof)
        ));
    }

    #[test]
    fn proof_from_another_agent_is_rejected() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let intruder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 42});
        let cwt = permit_for(&signer, "github.merge_pr", &args);
        let proof = prove_invocation(
            &intruder,
            &cwt,
            "github.merge_pr",
            &args,
            HashAlgorithm::Sha256,
        );
        let permit = verify(&cwt, NOW).unwrap();
        assert!(matches!(
            verify_invocation_proof(
                &permit,
                &cwt,
                "github.merge_pr",
                &args,
                &proof,
                HashAlgorithm::Sha256
            ),
            Err(PermitError::InvalidProof)
        ));
    }

    #[test]
    fn digest_is_domain_separated_and_deterministic() {
        let args = json!({"b": 1, "a": 2});
        let d1 = invocation_digest(b"permit", "t", &args, HashAlgorithm::Sha256);
        let d2 = invocation_digest(
            b"permit",
            "t",
            &json!({"a": 2, "b": 1}),
            HashAlgorithm::Sha256,
        );
        assert_eq!(d1, d2);
        assert_ne!(
            d1,
            invocation_digest(b"permit", "u", &args, HashAlgorithm::Sha256)
        );
        assert_ne!(
            d1,
            invocation_digest(b"other", "t", &args, HashAlgorithm::Sha256)
        );
        assert_eq!(d1.len(), 32);
    }
}
