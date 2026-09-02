//! Proof-of-possession over one invocation. The permit binds tool and
//! argument hash; the proof shows the caller holds the `cnf` private key
//! for exactly this permit and exactly these arguments, so a captured
//! permit is useless to anyone else and to the same agent with other args.
//!
//! The digest names the permit by [`Permit::id`], the digest of its signed
//! content, not by its wire bytes. One issuance has many valid encodings —
//! tagged or bare, and ECDSA signatures are malleable — and keying the proof
//! on bytes made a re-encoding demand a fresh proof while the budget counter,
//! keyed on the identity, kept counting the same permit. Both now name the
//! permit the same way.
//!
//! A gate composes three calls, in this order:
//!
//! 1. [`verify`](crate::verify) — the permit's issuer, signature, and
//!    validity window, yielding a [`Permit`] and its `id`. Never
//!    [`decode`](crate::decode), which checks neither issuer nor signature.
//! 2. [`verify_invocation_proof`] — that the presenter holds the `cnf` key
//!    and signed *this* invocation.
//! 3. [`PermitClaims::verify_invocation`](crate::PermitClaims::verify_invocation)
//!    — that the invocation is the one the permit authorizes.
//!
//! `arkavo-dispatch-gate` is the implementation of that composition.

use crate::canonical::argument_hash;
use crate::error::PermitError;
use crate::hash::HashAlgorithm;
use crate::keys::PermitSigner;
use crate::permit::Permit;
use serde_json::Value;

const DOMAIN: &[u8] = b"arkavo-permit-pop/v1";

/// The bytes a presenter signs to prove this invocation of this permit.
///
/// `permit_id` is [`Permit::id`]: callers take it from
/// [`verify`](crate::verify) on the authorizing path, or from
/// [`decode`](crate::decode) when merely minting a proof for a permit they
/// already hold.
pub fn invocation_digest(
    permit_id: &[u8; 32],
    tool_name: &str,
    arguments: &Value,
    algorithm: HashAlgorithm,
) -> Vec<u8> {
    let mut input = Vec::with_capacity(DOMAIN.len() + 32 + 8 + tool_name.len() + 32);
    input.extend_from_slice(DOMAIN);
    input.extend_from_slice(permit_id);
    input.extend_from_slice(&(tool_name.len() as u64).to_be_bytes());
    input.extend_from_slice(tool_name.as_bytes());
    input.extend_from_slice(&argument_hash(arguments, algorithm));
    algorithm.digest(&input)
}

/// Sign [`invocation_digest`] with the presenter's `cnf` key.
pub fn prove_invocation(
    signer: &PermitSigner,
    permit_id: &[u8; 32],
    tool_name: &str,
    arguments: &Value,
    algorithm: HashAlgorithm,
) -> Vec<u8> {
    signer.sign(&invocation_digest(
        permit_id, tool_name, arguments, algorithm,
    ))
}

/// Check a proof against the permit's confirmation key.
///
/// The permit carries its own identity, so this needs no token bytes: a
/// proof made for one issuance verifies against every encoding of it.
pub fn verify_invocation_proof(
    permit: &Permit,
    tool_name: &str,
    arguments: &Value,
    proof: &[u8],
    algorithm: HashAlgorithm,
) -> Result<(), PermitError> {
    let digest = invocation_digest(&permit.id, tool_name, arguments, algorithm);
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
    use crate::{argument_hash, decode, mint, verify};
    use arkavo_crypto::{AgentKeypair, P256SigningKeypair};
    use arkavo_test_macros::spec;
    use serde_json::json;

    const NOW: i64 = 1_700_000_060;

    fn ed25519() -> PermitSigner {
        PermitSigner::Ed25519(AgentKeypair::generate())
    }

    fn p256() -> PermitSigner {
        PermitSigner::P256(P256SigningKeypair::generate())
    }

    fn permit_for(
        issuer: &PermitSigner,
        holder: &PermitSigner,
        tool: &str,
        args: &serde_json::Value,
    ) -> Vec<u8> {
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
        mint(&claims, issuer, &holder.public_key()).unwrap()
    }

    #[test]
    fn proof_from_the_cnf_key_verifies() {
        let issuer = ed25519();
        let holder = ed25519();
        let args = json!({"pr": 42});
        let cwt = permit_for(&issuer, &holder, "github.merge_pr", &args);
        let id = decode(&cwt).unwrap().id;
        let proof = prove_invocation(
            &holder,
            &id,
            "github.merge_pr",
            &args,
            HashAlgorithm::Sha256,
        );
        let permit = verify(&cwt, NOW, &[issuer.public_key()]).unwrap();
        verify_invocation_proof(
            &permit,
            "github.merge_pr",
            &args,
            &proof,
            HashAlgorithm::Sha256,
        )
        .unwrap();
    }

    /// ES256 issuer, P-256 presenter: the whole permit exercised on the NIST
    /// curve, where the signature encoding differs from Ed25519's.
    #[test]
    fn p256_proof_from_the_cnf_key_verifies() {
        let issuer = p256();
        let holder = p256();
        let args = json!({"argv": ["ls", "-la"]});
        let cwt = permit_for(&issuer, &holder, "arkavo.shell.exec", &args);
        let id = decode(&cwt).unwrap().id;
        let proof = prove_invocation(
            &holder,
            &id,
            "arkavo.shell.exec",
            &args,
            HashAlgorithm::Sha256,
        );
        assert_eq!(proof.len(), 64, "ES256 proofs are raw r || s");

        let permit = verify(&cwt, NOW, &[issuer.public_key()]).unwrap();
        verify_invocation_proof(
            &permit,
            "arkavo.shell.exec",
            &args,
            &proof,
            HashAlgorithm::Sha256,
        )
        .unwrap();

        // And a P-256 proof from anyone else still fails.
        let intruder = p256();
        let forged = prove_invocation(
            &intruder,
            &id,
            "arkavo.shell.exec",
            &args,
            HashAlgorithm::Sha256,
        );
        assert!(matches!(
            verify_invocation_proof(
                &permit,
                "arkavo.shell.exec",
                &args,
                &forged,
                HashAlgorithm::Sha256
            ),
            Err(PermitError::InvalidProof)
        ));
    }

    #[test]
    #[spec("PDG-003")]
    fn replay_with_different_arguments_is_rejected() {
        let issuer = ed25519();
        let holder = ed25519();
        let args = json!({"pr": 42});
        let cwt = permit_for(&issuer, &holder, "github.merge_pr", &args);
        let id = decode(&cwt).unwrap().id;
        let proof = prove_invocation(
            &holder,
            &id,
            "github.merge_pr",
            &args,
            HashAlgorithm::Sha256,
        );
        let permit = verify(&cwt, NOW, &[issuer.public_key()]).unwrap();
        let other = json!({"pr": 43});
        assert!(matches!(
            verify_invocation_proof(
                &permit,
                "github.merge_pr",
                &other,
                &proof,
                HashAlgorithm::Sha256
            ),
            Err(PermitError::InvalidProof)
        ));
    }

    #[test]
    #[spec("PDG-003")]
    fn proof_from_another_agent_is_rejected() {
        let issuer = ed25519();
        let holder = ed25519();
        let intruder = ed25519();
        let args = json!({"pr": 42});
        let cwt = permit_for(&issuer, &holder, "github.merge_pr", &args);
        let id = decode(&cwt).unwrap().id;
        let proof = prove_invocation(
            &intruder,
            &id,
            "github.merge_pr",
            &args,
            HashAlgorithm::Sha256,
        );
        let permit = verify(&cwt, NOW, &[issuer.public_key()]).unwrap();
        assert!(matches!(
            verify_invocation_proof(
                &permit,
                "github.merge_pr",
                &args,
                &proof,
                HashAlgorithm::Sha256
            ),
            Err(PermitError::InvalidProof)
        ));
    }

    /// A proof names the permit by its signed identity, so re-encoding the
    /// token — dropping tag 18 here — leaves the proof valid. Keying it on
    /// the wire bytes instead put the proof and the gate's budget counter on
    /// two different notions of "the same permit".
    #[test]
    #[spec("PDG-004")]
    fn a_proof_survives_re_encoding_of_its_permit() {
        use coset::{CborSerializable as _, CoseSign1, TaggedCborSerializable as _};

        let issuer = ed25519();
        let holder = ed25519();
        let args = json!({"pr": 42});
        let cwt = permit_for(&issuer, &holder, "github.merge_pr", &args);
        let id = decode(&cwt).unwrap().id;
        let proof = prove_invocation(
            &holder,
            &id,
            "github.merge_pr",
            &args,
            HashAlgorithm::Sha256,
        );

        let sign1 = CoseSign1::from_tagged_slice(&cwt[2..]).unwrap();
        let mut bare = crate::CWT_TAG_PREFIX.to_vec();
        bare.extend_from_slice(&sign1.to_vec().unwrap());
        assert_ne!(bare, cwt, "the re-encoding must differ in bytes");

        let reencoded = verify(&bare, NOW, &[issuer.public_key()]).unwrap();
        assert_eq!(reencoded.id, id);
        verify_invocation_proof(
            &reencoded,
            "github.merge_pr",
            &args,
            &proof,
            HashAlgorithm::Sha256,
        )
        .expect("one proof covers every encoding of one issuance");
    }

    #[test]
    #[spec("PDG-003")]
    fn digest_is_domain_separated_and_deterministic() {
        let args = json!({"b": 1, "a": 2});
        let id = [0x11u8; 32];
        let other_id = [0x22u8; 32];
        let d1 = invocation_digest(&id, "t", &args, HashAlgorithm::Sha256);
        let d2 = invocation_digest(&id, "t", &json!({"a": 2, "b": 1}), HashAlgorithm::Sha256);
        assert_eq!(d1, d2);
        assert_ne!(
            d1,
            invocation_digest(&id, "u", &args, HashAlgorithm::Sha256)
        );
        assert_ne!(
            d1,
            invocation_digest(&other_id, "t", &args, HashAlgorithm::Sha256)
        );
        assert_eq!(d1.len(), 32);
    }
}
