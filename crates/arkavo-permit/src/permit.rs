//! Minting, decoding, and verifying CWT permits.
//!
//! Wire format: CBOR tag 61 (CWT) wrapping a tagged COSE_Sign1 (tag 18)
//! whose payload is the CBOR-encoded claims set. The protected header carries
//! the signing algorithm and a content type of `application/cwt`.

use crate::claims::PermitClaims;
use crate::error::PermitError;
use crate::keys::{PermitSigner, PermitVerifier};
use ciborium::value::Value;
use coset::iana::{Algorithm, CoapContentFormat};
use coset::{
    CoseSign1, CoseSign1Builder, HeaderBuilder, RegisteredLabelWithPrivate, TaggedCborSerializable,
};

/// CBOR tag 61 (CWT) prefix bytes: 0xd8 0x3d.
const CWT_TAG_PREFIX: [u8; 2] = [0xd8, 0x3d];

/// Permits are small CWTs. Untrusted `decode`/`verify` input larger than this
/// is rejected before COSE/CBOR parse.
pub const MAX_PERMIT_BYTES: usize = 16 * 1024;

/// A decoded permit: the validated claims plus the confirmation key they
/// were bound to. Instances returned by [`verify`] are additionally
/// signature- and time-checked.
pub struct Permit {
    pub claims: PermitClaims,
    pub confirmation_key: PermitVerifier,
}

/// Mint a signed permit CWT. The `cnf` claim is derived from the signer.
pub fn mint(claims: &PermitClaims, signer: &PermitSigner) -> Result<Vec<u8>, PermitError> {
    claims.validate()?;
    let claims_value = claims.to_cbor_value(&signer.cose_key())?;
    let mut payload = Vec::new();
    ciborium::into_writer(&claims_value, &mut payload)
        .map_err(|e| PermitError::CborSerialize(format!("claims set: {e}")))?;
    let header = HeaderBuilder::new()
        .algorithm(signer.algorithm())
        .content_format(CoapContentFormat::Cwt)
        .build();
    let sign1 = CoseSign1Builder::new()
        .protected(header)
        .payload(payload)
        .create_signature(&[], |data| signer.sign(data))
        .build();
    let sign1_bytes = sign1
        .to_tagged_vec()
        .map_err(|e| PermitError::Cose(format!("COSE_Sign1 encode: {e}")))?;
    let mut out = Vec::with_capacity(CWT_TAG_PREFIX.len() + sign1_bytes.len());
    out.extend_from_slice(&CWT_TAG_PREFIX);
    out.extend_from_slice(&sign1_bytes);
    Ok(out)
}

/// Decode a permit without verifying the signature or validity window.
///
/// The claims are structurally validated (fail-closed on malformed input),
/// but callers must not make authorization decisions from the result; use
/// [`verify`] for that.
///
/// Input larger than [`MAX_PERMIT_BYTES`] is rejected before parse.
pub fn decode(cwt: &[u8]) -> Result<Permit, PermitError> {
    let sign1 = parse_sign1(cwt)?;
    extract(&sign1)
}

/// Decode and fully verify a permit: structure, signature against the `cnf`
/// key, algorithm/key agreement, and the nbf/exp/iat window at `now`
/// (seconds since UNIX epoch).
///
/// Input larger than [`MAX_PERMIT_BYTES`] is rejected before parse.
pub fn verify(cwt: &[u8], now: i64) -> Result<Permit, PermitError> {
    let sign1 = parse_sign1(cwt)?;
    let algorithm = header_algorithm(&sign1)?;
    let permit = extract(&sign1)?;
    sign1.verify_signature(&[], |signature, data| {
        permit.confirmation_key.verify(algorithm, data, signature)
    })?;
    let claims = &permit.claims;
    if now < claims.not_before {
        return Err(PermitError::NotYetValid {
            nbf: claims.not_before,
            now,
        });
    }
    if now >= claims.expires_at {
        return Err(PermitError::Expired {
            exp: claims.expires_at,
            now,
        });
    }
    if claims.issued_at > now {
        return Err(PermitError::IssuedInFuture {
            iat: claims.issued_at,
            now,
        });
    }
    Ok(permit)
}

fn parse_sign1(cwt: &[u8]) -> Result<CoseSign1, PermitError> {
    if cwt.len() > MAX_PERMIT_BYTES {
        return Err(PermitError::Cose("permit exceeds maximum size".to_string()));
    }
    let tagged = cwt
        .strip_prefix(&CWT_TAG_PREFIX)
        .ok_or(PermitError::Cose("missing CBOR tag 61 (CWT)".to_string()))?;
    CoseSign1::from_tagged_slice(tagged)
        .map_err(|e| PermitError::Cose(format!("COSE_Sign1 decode: {e}")))
}

fn header_algorithm(sign1: &CoseSign1) -> Result<Algorithm, PermitError> {
    match &sign1.protected.header.alg {
        Some(RegisteredLabelWithPrivate::Assigned(alg @ (Algorithm::EdDSA | Algorithm::ES256))) => {
            Ok(*alg)
        }
        Some(other) => Err(PermitError::UnsupportedAlgorithm(format!("{other:?}"))),
        None => Err(PermitError::MalformedClaim("protected header alg")),
    }
}

fn extract(sign1: &CoseSign1) -> Result<Permit, PermitError> {
    let payload = sign1
        .payload
        .as_ref()
        .ok_or(PermitError::MalformedClaim("detached payload"))?;
    let value: Value = ciborium::from_reader(payload.as_slice())
        .map_err(|e| PermitError::CborDeserialize(format!("claims set: {e}")))?;
    let (claims, cose_key) = PermitClaims::from_cbor_value(&value)?;
    let confirmation_key = PermitVerifier::from_cose_key(&cose_key)?;
    Ok(Permit {
        claims,
        confirmation_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::{BUDGET_MAX_INVOCATIONS, Budget, CLAIM_CONFIRMATION, CNF_COSE_KEY};
    use crate::hash::HashAlgorithm;
    use crate::{argument_hash, keys::PermitVerifier as Verifier};
    use arkavo_crypto::{AgentKeypair, P256SigningKeypair};
    use coset::{AsCborValue, CborSerializable};

    const IAT: i64 = 1_700_000_000;
    const NOW: i64 = 1_700_000_060;
    const EXP: i64 = 1_700_000_300;

    fn sample_claims() -> PermitClaims {
        let arguments = serde_json::json!({"path": "/tmp/data.csv", "max_bytes": 4096});
        PermitClaims {
            issuer: "https://issuer.example".to_string(),
            subject: "did:example:alice".to_string(),
            agent_workload_id: "spiffe://edge/agent-1".to_string(),
            policy_bundle_hash: HashAlgorithm::Sha256.digest(b"policy-bundle-v1"),
            tool_name: "arkavo.fs.read".to_string(),
            argument_hash: argument_hash(&arguments, HashAlgorithm::Sha256),
            data_classifications: vec!["tdf:confidential".to_string()],
            budget: Budget {
                max_invocations: 5,
                token_ceiling: Some(100_000),
                cost_micro_usd: Some(250_000),
            },
            sequence_state_hash: HashAlgorithm::Sha256.digest(b"sequence-0"),
            issued_at: IAT,
            not_before: IAT,
            expires_at: EXP,
            parent_permit: None,
        }
    }

    fn ed25519_signer() -> PermitSigner {
        PermitSigner::Ed25519(AgentKeypair::generate())
    }

    #[test]
    fn roundtrip_ed25519() {
        let signer = ed25519_signer();
        let claims = sample_claims();
        let cwt = mint(&claims, &signer).unwrap();
        let permit = verify(&cwt, NOW).unwrap();
        assert_eq!(permit.claims, claims);
        assert_eq!(
            permit.confirmation_key.public_key_bytes(),
            signer.public_key().public_key_bytes()
        );
    }

    #[test]
    fn roundtrip_es256() {
        let signer = PermitSigner::P256(P256SigningKeypair::generate());
        let claims = sample_claims();
        let cwt = mint(&claims, &signer).unwrap();
        let permit = verify(&cwt, NOW).unwrap();
        assert_eq!(permit.claims, claims);
    }

    #[test]
    fn wire_format_starts_with_cwt_tag() {
        let cwt = mint(&sample_claims(), &ed25519_signer()).unwrap();
        // Tag 61 (CWT) then tag 18 (COSE_Sign1), both in canonical CBOR.
        assert_eq!(&cwt[..4], &[0xd8, 0x3d, 0xd2, 0x84], "tag 61 then tag 18");
    }

    #[test]
    fn tampered_signature_rejected() {
        let signer = ed25519_signer();
        let cwt = mint(&sample_claims(), &signer).unwrap();
        // Flip a bit in the final byte: Ed25519 signatures sit at the tail
        // of the COSE_Sign1 array.
        let mut tampered = cwt.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert!(matches!(
            verify(&tampered, NOW),
            Err(PermitError::InvalidSignature) | Err(PermitError::Cose(_))
        ));
    }

    #[test]
    fn tampered_payload_rejected() {
        let signer = ed25519_signer();
        let claims = sample_claims();
        let cwt = mint(&claims, &signer).unwrap();
        // Corrupt a byte in the middle of the claims payload.
        let mut tampered = cwt.clone();
        let mid = tampered.len() / 2;
        tampered[mid] ^= 0x40;
        assert!(verify(&tampered, NOW).is_err());
    }

    #[test]
    fn expired_permit_rejected() {
        let signer = ed25519_signer();
        let cwt = mint(&sample_claims(), &signer).unwrap();
        assert!(matches!(
            verify(&cwt, EXP),
            Err(PermitError::Expired { .. })
        ));
        assert!(matches!(
            verify(&cwt, EXP + 10_000),
            Err(PermitError::Expired { .. })
        ));
    }

    #[test]
    fn not_yet_valid_permit_rejected() {
        let signer = ed25519_signer();
        let mut claims = sample_claims();
        claims.not_before = NOW + 60;
        claims.issued_at = IAT;
        let cwt = mint(&claims, &signer).unwrap();
        assert!(matches!(
            verify(&cwt, NOW),
            Err(PermitError::NotYetValid { .. })
        ));
    }

    #[test]
    fn future_iat_rejected() {
        let signer = ed25519_signer();
        let mut claims = sample_claims();
        claims.issued_at = NOW + 60;
        let cwt = mint(&claims, &signer).unwrap();
        assert!(matches!(
            verify(&cwt, NOW),
            Err(PermitError::IssuedInFuture { .. })
        ));
    }

    #[test]
    fn wrong_cnf_key_rejected() {
        // Sign with key A but place key B in the cnf claim: the signature
        // cannot verify against the confirmation key.
        let signer_a = ed25519_signer();
        let signer_b = ed25519_signer();
        let claims = sample_claims();
        let mut value = claims.to_cbor_value(&signer_a.cose_key()).unwrap();
        if let Value::Map(entries) = &mut value {
            for (k, v) in entries.iter_mut() {
                if matches!(k, Value::Integer(i) if i128::from(*i) == CLAIM_CONFIRMATION as i128) {
                    let cnf_key = signer_b
                        .cose_key()
                        .to_cbor_value()
                        .expect("cnf key encodes");
                    *v = Value::Map(vec![(
                        Value::Integer(ciborium::value::Integer::from(CNF_COSE_KEY)),
                        cnf_key,
                    )]);
                }
            }
        }
        let mut payload = Vec::new();
        ciborium::into_writer(&value, &mut payload).unwrap();
        let header = HeaderBuilder::new().algorithm(Algorithm::EdDSA).build();
        let sign1 = CoseSign1Builder::new()
            .protected(header)
            .payload(payload)
            .create_signature(&[], |data| signer_a.sign(data))
            .build();
        let sign1_bytes = sign1.to_tagged_vec().unwrap();
        let mut cwt = CWT_TAG_PREFIX.to_vec();
        cwt.extend_from_slice(&sign1_bytes);
        assert!(matches!(
            verify(&cwt, NOW),
            Err(PermitError::InvalidSignature)
        ));
    }

    #[test]
    fn canonical_argument_hash_mismatch_rejected() {
        let signer = ed25519_signer();
        let claims = sample_claims();
        let cwt = mint(&claims, &signer).unwrap();
        let permit = verify(&cwt, NOW).unwrap();

        let expected_args = serde_json::json!({"max_bytes": 4096, "path": "/tmp/data.csv"});
        assert!(
            permit
                .claims
                .verify_invocation("arkavo.fs.read", &expected_args, HashAlgorithm::Sha256)
                .is_ok()
        );

        let wrong_args = serde_json::json!({"max_bytes": 8192, "path": "/tmp/data.csv"});
        assert!(matches!(
            permit
                .claims
                .verify_invocation("arkavo.fs.read", &wrong_args, HashAlgorithm::Sha256),
            Err(PermitError::BindingMismatch(_))
        ));
    }

    #[test]
    fn unknown_claims_are_tolerated() {
        let signer = ed25519_signer();
        let claims = sample_claims();
        let mut value = claims.to_cbor_value(&signer.cose_key()).unwrap();
        if let Value::Map(entries) = &mut value {
            entries.push((
                Value::Integer(ciborium::value::Integer::from(-79999)),
                Value::Text("future extension".to_string()),
            ));
        }
        let mut payload = Vec::new();
        ciborium::into_writer(&value, &mut payload).unwrap();
        let sign1 = CoseSign1Builder::new()
            .protected(HeaderBuilder::new().algorithm(Algorithm::EdDSA).build())
            .payload(payload)
            .create_signature(&[], |data| signer.sign(data))
            .build();
        let mut cwt = CWT_TAG_PREFIX.to_vec();
        cwt.extend_from_slice(&sign1.to_tagged_vec().unwrap());
        let permit = verify(&cwt, NOW).unwrap();
        assert_eq!(permit.claims, claims);
    }

    #[test]
    fn parent_permit_chain_roundtrip() {
        let parent_signer = ed25519_signer();
        let parent = mint(&sample_claims(), &parent_signer).unwrap();

        let child_signer = ed25519_signer();
        let mut child_claims = sample_claims();
        child_claims.tool_name = "arkavo.a2a.delegate".to_string();
        child_claims.parent_permit = Some(HashAlgorithm::Sha256.digest(&parent));
        let child = mint(&child_claims, &child_signer).unwrap();

        let permit = verify(&child, NOW).unwrap();
        // The parent hash binds the child to the exact parent CWT bytes.
        assert_eq!(
            permit.claims.parent_permit.as_deref(),
            Some(HashAlgorithm::Sha256.digest(&parent).as_slice())
        );
    }

    #[test]
    fn oversized_permit_rejected_before_parse() {
        let oversized = vec![0u8; MAX_PERMIT_BYTES + 1];
        assert!(matches!(
            decode(&oversized),
            Err(PermitError::Cose(msg)) if msg.contains("maximum size")
        ));
        assert!(matches!(
            verify(&oversized, NOW),
            Err(PermitError::Cose(msg)) if msg.contains("maximum size")
        ));
    }

    #[test]
    fn decode_does_not_verify_but_validates_structure() {
        let signer = ed25519_signer();
        let claims = sample_claims();
        let cwt = mint(&claims, &signer).unwrap();
        // Expired at this instant, but decode must still succeed.
        let permit = decode(&cwt).unwrap();
        assert_eq!(permit.claims, claims);
        assert!(decode(&cwt[..10]).is_err());
        assert!(decode(b"\xd8\x3dgarbage").is_err());
    }

    #[test]
    fn missing_budget_field_rejected() {
        let signer = ed25519_signer();
        let claims = sample_claims();
        let mut value = claims.to_cbor_value(&signer.cose_key()).unwrap();
        if let Value::Map(entries) = &mut value {
            for (k, v) in entries.iter_mut() {
                let is_budget = matches!(k, Value::Integer(i) if i128::from(*i) == -70006);
                if is_budget && let Value::Map(budget) = v {
                    budget.retain(|(bk, _)| {
                        !matches!(bk, Value::Integer(i) if i128::from(*i) == BUDGET_MAX_INVOCATIONS as i128)
                    });
                }
            }
        }
        let mut payload = Vec::new();
        ciborium::into_writer(&value, &mut payload).unwrap();
        let sign1 = CoseSign1Builder::new()
            .protected(HeaderBuilder::new().algorithm(Algorithm::EdDSA).build())
            .payload(payload)
            .create_signature(&[], |data| signer.sign(data))
            .build();
        let mut cwt = CWT_TAG_PREFIX.to_vec();
        cwt.extend_from_slice(&sign1.to_tagged_vec().unwrap());
        assert!(matches!(
            verify(&cwt, NOW),
            Err(PermitError::MissingClaim("budget.max_invocations"))
        ));
    }

    #[test]
    fn cnf_key_recovers_expected_verifier() {
        let signer = ed25519_signer();
        let cwt = mint(&sample_claims(), &signer).unwrap();
        let permit = decode(&cwt).unwrap();
        let _expected: Verifier = signer.public_key();
        assert_eq!(
            permit.confirmation_key.public_key_bytes(),
            _expected.public_key_bytes()
        );
    }

    #[test]
    fn cbor_serializable_sign1_roundtrip_used_internally() {
        let signer = ed25519_signer();
        let cwt = mint(&sample_claims(), &signer).unwrap();
        let sign1 = parse_sign1(&cwt).unwrap();
        let bytes = sign1.clone().to_vec().unwrap();
        let parsed = CoseSign1::from_slice(&bytes).unwrap();
        assert_eq!(parsed.signature, sign1.signature);
    }
}
