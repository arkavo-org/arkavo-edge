//! Minting, decoding, and verifying CWT permits.
//!
//! Wire format: CBOR tag 61 (CWT) wrapping a COSE_Sign1 whose payload is the
//! CBOR-encoded claims set. [`mint`] emits the COSE_Sign1 tagged (tag 18);
//! the shared `arkavo-cwt` parser also accepts it bare, as authnz-rs emits
//! it. The protected header carries the signing algorithm, the issuer's key
//! identifier, and a content type of `application/cwt`; the unprotected
//! header must be empty. Because those encodings differ in bytes but not in
//! what was signed, a permit's identity is [`Permit::id`] — a digest of the
//! signed Sig_structure — and never a hash of the token bytes.
//!
//! The issuer signs the permit and the `cnf` claim names the presenter's key
//! (RFC 8747), so verifiers hold a list of trusted issuer keys: a permit
//! signed by the key it confirms proves nothing about authority.

use crate::claims::PermitClaims;
use crate::error::PermitError;
use crate::keys::{PermitSigner, PermitVerifier};
use ciborium::value::Value;
use coset::iana::CoapContentFormat;
use coset::{CoseSign1Builder, HeaderBuilder, TaggedCborSerializable};
use sha2::{Digest, Sha256};

// The CWT envelope belongs to one place. `arkavo-cwt` declares the tag-61
// prefix and the size cap, parses against them, and permits mint and verify
// against the same values rather than a second copy that could drift.
use arkavo_cwt::sign1::CWT_TAG_PREFIX;

/// Permits are small CWTs. Untrusted `decode`/`verify` input larger than this
/// is rejected before COSE/CBOR parse.
///
/// The same bound `arkavo-cwt` enforces on every token it parses, named here
/// for callers who think in permits.
pub use arkavo_cwt::sign1::MAX_TOKEN_BYTES as MAX_PERMIT_BYTES;

/// A decoded permit: the validated claims plus the presenter's confirmation
/// key from the `cnf` claim. Instances returned by [`verify`] are
/// additionally issuer-, signature- and time-checked.
pub struct Permit {
    pub claims: PermitClaims,
    pub confirmation_key: PermitVerifier,
    /// The permit's stable identity: SHA-256 over the COSE Sig_structure the
    /// issuer signed (protected header plus payload), never over the wire
    /// bytes.
    ///
    /// The wire bytes are not an identity. The unprotected header is outside
    /// the signature, the COSE_Sign1 parses whether tagged or bare, and ECDSA
    /// signatures are malleable, so a holder can re-encode one issuance into
    /// unboundedly many distinct byte strings that all still verify. Every
    /// byte this digest covers is signed, so all re-encodings of one issuance
    /// share an `id` while two distinct issuances do not. Callers keeping
    /// per-permit state — a budget counter, a replay record — must key it on
    /// this, never on a hash of the token bytes.
    pub id: [u8; 32],
}

/// The key identifier an issuer is named by on the wire: SHA-256 over the
/// issuer's raw public key bytes. [`mint`] writes it into the protected
/// header and [`verify`] looks the issuer up by it.
pub fn issuer_kid(issuer: &PermitVerifier) -> [u8; 32] {
    Sha256::digest(issuer.public_key_bytes()).into()
}

/// Mint a permit CWT signed by `issuer`, confirming `confirmation_key` as the
/// presenter's proof-of-possession key (RFC 8747).
///
/// The two keys are distinct roles: the issuer's key is the authority a
/// verifier trusts, the confirmation key belongs to whoever will present the
/// permit. Passing the same key for both produces a permit no verifier
/// accepts unless that key is on its trusted issuer list.
pub fn mint(
    claims: &PermitClaims,
    issuer: &PermitSigner,
    confirmation_key: &PermitVerifier,
) -> Result<Vec<u8>, PermitError> {
    claims.validate()?;
    let claims_value = claims.to_cbor_value(&confirmation_key.to_cose_key())?;
    let mut payload = Vec::new();
    ciborium::into_writer(&claims_value, &mut payload)
        .map_err(|e| PermitError::CborSerialize(format!("claims set: {e}")))?;
    let header = HeaderBuilder::new()
        .algorithm(issuer.algorithm())
        .key_id(issuer_kid(&issuer.public_key()).to_vec())
        .content_format(CoapContentFormat::Cwt)
        .build();
    let sign1 = CoseSign1Builder::new()
        .protected(header)
        .payload(payload)
        .create_signature(&[], |data| issuer.sign(data))
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
/// but the issuer is neither identified nor trusted and the signature is not
/// checked, so callers must not make authorization decisions from the
/// result; use [`verify`] for that.
///
/// Input larger than [`MAX_PERMIT_BYTES`] is rejected before parse.
pub fn decode(cwt: &[u8]) -> Result<Permit, PermitError> {
    let parsed = parse_sign1(cwt)?;
    extract(&parsed)
}

/// Decode and fully verify a permit.
///
/// Checks its structure, an issuer drawn from `trusted_issuers` by the
/// protected header's `kid`, that issuer's signature, and the nbf/exp/iat
/// window at `now` (seconds since UNIX epoch).
///
/// The returned [`Permit::confirmation_key`] is the presenter's key from the
/// `cnf` claim; proving possession of it is a separate step.
///
/// Checks run in order: size cap, CWT tag, COSE parse, empty unprotected
/// header, issuer lookup, signature, claim extraction, validity window. A
/// permit from an unknown issuer is refused before its claims are parsed.
///
/// The unprotected header must be empty: nothing is expected there and it
/// sits outside the signature, so refusing it stops a holder re-encoding one
/// issuance into fresh, still-valid byte strings. That is belt and braces
/// alongside [`Permit::id`], which digests signed bytes only.
///
/// Input larger than [`MAX_PERMIT_BYTES`] is rejected before parse.
pub fn verify(
    cwt: &[u8],
    now: i64,
    trusted_issuers: &[PermitVerifier],
) -> Result<Permit, PermitError> {
    let parsed = parse_sign1(cwt)?;
    if !parsed.sign1.unprotected.is_empty() {
        return Err(PermitError::Cose(
            "unprotected header must be empty".to_string(),
        ));
    }
    let issuer = find_trusted_issuer(&parsed, trusted_issuers)?;
    parsed.verify(&issuer.0)?;
    let permit = extract(&parsed)?;
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

/// The trusted issuer named by the token's `kid`, or
/// [`PermitError::UntrustedIssuer`].
///
/// A candidate must match both the key identifier and the header algorithm,
/// so a header cannot borrow a trusted issuer's identity under an algorithm
/// that issuer does not sign with.
fn find_trusted_issuer<'a>(
    parsed: &arkavo_cwt::ParsedSign1,
    trusted_issuers: &'a [PermitVerifier],
) -> Result<&'a PermitVerifier, PermitError> {
    let kid = parsed.kid();
    trusted_issuers
        .iter()
        .find(|issuer| {
            issuer_kid(issuer).as_slice() == kid && issuer.algorithm() == parsed.algorithm
        })
        .ok_or(PermitError::UntrustedIssuer)
}

fn parse_sign1(cwt: &[u8]) -> Result<arkavo_cwt::ParsedSign1, PermitError> {
    if cwt.len() > MAX_PERMIT_BYTES {
        return Err(PermitError::Cose("permit exceeds maximum size".to_string()));
    }
    if !cwt.starts_with(&CWT_TAG_PREFIX) {
        return Err(PermitError::Cose("missing CBOR tag 61 (CWT)".to_string()));
    }
    Ok(arkavo_cwt::sign1::parse(cwt)?)
}

fn extract(parsed: &arkavo_cwt::ParsedSign1) -> Result<Permit, PermitError> {
    let payload = parsed.payload()?;
    let value: Value = ciborium::from_reader(payload)
        .map_err(|e| PermitError::CborDeserialize(format!("claims set: {e}")))?;
    let (claims, cose_key) = PermitClaims::from_cbor_value(&value)?;
    let confirmation_key = PermitVerifier::from_cose_key(&cose_key)?;
    // `tbs_data` rebuilds the Sig_structure the issuer signed, so the digest
    // covers signed bytes only and is identical for every re-encoding of one
    // permit.
    let id = Sha256::digest(parsed.sign1.tbs_data(b"")).into();
    Ok(Permit {
        claims,
        confirmation_key,
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::argument_hash;
    use crate::claims::{BUDGET_MAX_INVOCATIONS, Budget};
    use crate::hash::HashAlgorithm;
    use arkavo_crypto::{AgentKeypair, P256SigningKeypair};
    use coset::iana::Algorithm;
    use coset::{CborSerializable, CoseSign1};

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

    fn p256_signer() -> PermitSigner {
        PermitSigner::P256(P256SigningKeypair::generate())
    }

    /// The claims set as a CBOR value, so tests can perturb it before signing.
    fn claims_value(claims: &PermitClaims, confirmation_key: &PermitVerifier) -> Value {
        claims
            .to_cbor_value(&confirmation_key.to_cose_key())
            .expect("claims encode")
    }

    fn encode(value: &Value) -> Vec<u8> {
        let mut payload = Vec::new();
        ciborium::into_writer(value, &mut payload).expect("payload encodes");
        payload
    }

    /// Assemble a permit by hand so a test can choose the protected header
    /// and the signing key independently of [`mint`].
    fn hand_built(
        payload: Vec<u8>,
        algorithm: Algorithm,
        kid: &[u8],
        signer: &PermitSigner,
    ) -> Vec<u8> {
        let header = HeaderBuilder::new()
            .algorithm(algorithm)
            .key_id(kid.to_vec())
            .build();
        let sign1 = CoseSign1Builder::new()
            .protected(header)
            .payload(payload)
            .create_signature(&[], |data| signer.sign(data))
            .build();
        let mut cwt = CWT_TAG_PREFIX.to_vec();
        cwt.extend_from_slice(&sign1.to_tagged_vec().expect("sign1 encodes"));
        cwt
    }

    #[test]
    fn roundtrip_ed25519() {
        let issuer = ed25519_signer();
        let holder = ed25519_signer();
        let claims = sample_claims();
        let cwt = mint(&claims, &issuer, &holder.public_key()).unwrap();
        let permit = verify(&cwt, NOW, &[issuer.public_key()]).unwrap();
        assert_eq!(permit.claims, claims);
        assert_eq!(
            permit.confirmation_key.public_key_bytes(),
            holder.public_key().public_key_bytes(),
            "cnf must name the presenter, not the issuer"
        );
    }

    #[test]
    fn roundtrip_es256() {
        let issuer = p256_signer();
        let holder = p256_signer();
        let claims = sample_claims();
        let cwt = mint(&claims, &issuer, &holder.public_key()).unwrap();
        let permit = verify(&cwt, NOW, &[issuer.public_key()]).unwrap();
        assert_eq!(permit.claims, claims);
        assert_eq!(
            permit.confirmation_key.public_key_bytes(),
            holder.public_key().public_key_bytes()
        );
    }

    /// RFC 8152 section 8.1: an ES256 signature value is the fixed-size
    /// 64-byte `r || s`, never DER. The signer emits that encoding directly,
    /// so no conversion sits on the minting path to fail quietly and leave a
    /// permit whose signature no verifier accepts.
    #[test]
    fn es256_permit_carries_a_fixed_size_signature() {
        let issuer = p256_signer();
        let holder = p256_signer();
        let cwt = mint(&sample_claims(), &issuer, &holder.public_key()).unwrap();
        let parsed = parse_sign1(&cwt).unwrap();
        assert_eq!(parsed.algorithm, Algorithm::ES256);
        assert_eq!(
            parsed.sign1.signature.len(),
            64,
            "COSE ES256 requires raw r || s"
        );
        verify(&cwt, NOW, &[issuer.public_key()]).expect("and it verifies");
    }

    #[test]
    fn issuer_and_confirmation_algorithms_are_independent() {
        // RFC 8747 puts no relationship between the algorithm the issuer
        // signs with and the algorithm of the presenter's key.
        let issuer = ed25519_signer();
        let holder = p256_signer();
        let cwt = mint(&sample_claims(), &issuer, &holder.public_key()).unwrap();
        let permit = verify(&cwt, NOW, &[issuer.public_key()]).unwrap();
        assert_eq!(permit.confirmation_key.algorithm(), Algorithm::ES256);
    }

    #[test]
    fn mint_records_the_issuer_kid_in_the_protected_header() {
        let issuer = ed25519_signer();
        let holder = ed25519_signer();
        let cwt = mint(&sample_claims(), &issuer, &holder.public_key()).unwrap();
        let parsed = parse_sign1(&cwt).unwrap();
        assert_eq!(parsed.kid(), issuer_kid(&issuer.public_key()));
        assert_ne!(parsed.kid(), issuer_kid(&holder.public_key()));
    }

    #[test]
    fn issuer_kid_is_sha256_of_public_key_bytes() {
        let issuer = ed25519_signer();
        let key = issuer.public_key();
        assert_eq!(
            issuer_kid(&key).as_slice(),
            HashAlgorithm::Sha256
                .digest(&key.public_key_bytes())
                .as_slice()
        );
        assert_ne!(issuer_kid(&key), issuer_kid(&ed25519_signer().public_key()));
    }

    #[test]
    fn self_minted_permit_is_untrusted() {
        // The defect this model closes: holding a keypair is not authority to
        // issue. A permit signed by its own cnf holder must be refused by a
        // verifier whose trusted list does not name that key.
        let rogue = ed25519_signer();
        let cwt = mint(&sample_claims(), &rogue, &rogue.public_key()).unwrap();
        let trusted = [ed25519_signer().public_key(), p256_signer().public_key()];
        assert!(matches!(
            verify(&cwt, NOW, &trusted),
            Err(PermitError::UntrustedIssuer)
        ));
        assert!(matches!(
            verify(&cwt, NOW, &[]),
            Err(PermitError::UntrustedIssuer)
        ));
    }

    #[test]
    fn cnf_key_alone_does_not_authorize() {
        // Trusting the presenter's key must not make its permits verify: the
        // signature is the issuer's, and the kid names the issuer.
        let issuer = ed25519_signer();
        let holder = ed25519_signer();
        let cwt = mint(&sample_claims(), &issuer, &holder.public_key()).unwrap();
        assert!(matches!(
            verify(&cwt, NOW, &[holder.public_key()]),
            Err(PermitError::UntrustedIssuer)
        ));
    }

    #[test]
    fn issuer_selected_by_kid_among_several() {
        let first = ed25519_signer();
        let second = p256_signer();
        let third = ed25519_signer();
        let holder = ed25519_signer();
        let cwt = mint(&sample_claims(), &second, &holder.public_key()).unwrap();
        let trusted = [first.public_key(), second.public_key(), third.public_key()];
        let permit = verify(&cwt, NOW, &trusted).unwrap();
        assert_eq!(permit.claims, sample_claims());
    }

    #[test]
    fn wrong_algorithm_issuer_with_same_kid_is_rejected() {
        // Only reachable by construction: `kid` is a SHA-256 digest, so two
        // keys of different algorithms never share one. A forged header can
        // still claim a trusted issuer's kid under the wrong `alg`.
        let issuer = ed25519_signer();
        let holder = ed25519_signer();
        let payload = encode(&claims_value(&sample_claims(), &holder.public_key()));
        let cwt = hand_built(
            payload,
            Algorithm::ES256,
            &issuer_kid(&issuer.public_key()),
            &issuer,
        );
        assert!(matches!(
            verify(&cwt, NOW, &[issuer.public_key()]),
            Err(PermitError::UntrustedIssuer)
        ));
    }

    #[test]
    fn untrusted_issuer_checked_before_claim_extraction() {
        // An untrusted token must learn nothing about its payload: the kid
        // lookup runs before the claims are parsed.
        let stranger = ed25519_signer();
        let cwt = hand_built(
            b"not a claims set".to_vec(),
            Algorithm::EdDSA,
            &issuer_kid(&stranger.public_key()),
            &stranger,
        );
        assert!(matches!(
            verify(&cwt, NOW, &[ed25519_signer().public_key()]),
            Err(PermitError::UntrustedIssuer)
        ));
    }

    #[test]
    fn wire_format_starts_with_cwt_tag() {
        let issuer = ed25519_signer();
        let cwt = mint(&sample_claims(), &issuer, &ed25519_signer().public_key()).unwrap();
        // Tag 61 (CWT) then tag 18 (COSE_Sign1), both in canonical CBOR.
        assert_eq!(&cwt[..4], &[0xd8, 0x3d, 0xd2, 0x84], "tag 61 then tag 18");
    }

    #[test]
    fn tampered_signature_rejected() {
        let issuer = ed25519_signer();
        let cwt = mint(&sample_claims(), &issuer, &ed25519_signer().public_key()).unwrap();
        // Flip a bit in the final byte: Ed25519 signatures sit at the tail
        // of the COSE_Sign1 array.
        let mut tampered = cwt;
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert!(matches!(
            verify(&tampered, NOW, &[issuer.public_key()]),
            Err(PermitError::InvalidSignature | PermitError::Cose(_))
        ));
    }

    #[test]
    fn tampered_payload_rejected() {
        let issuer = ed25519_signer();
        let cwt = mint(&sample_claims(), &issuer, &ed25519_signer().public_key()).unwrap();
        // Corrupt a byte in the middle of the claims payload.
        let mut tampered = cwt;
        let mid = tampered.len() / 2;
        tampered[mid] ^= 0x40;
        assert!(verify(&tampered, NOW, &[issuer.public_key()]).is_err());
    }

    #[test]
    fn expired_permit_rejected() {
        let issuer = ed25519_signer();
        let cwt = mint(&sample_claims(), &issuer, &ed25519_signer().public_key()).unwrap();
        let trusted = [issuer.public_key()];
        assert!(matches!(
            verify(&cwt, EXP, &trusted),
            Err(PermitError::Expired { .. })
        ));
        assert!(matches!(
            verify(&cwt, EXP + 10_000, &trusted),
            Err(PermitError::Expired { .. })
        ));
    }

    #[test]
    fn not_yet_valid_permit_rejected() {
        let issuer = ed25519_signer();
        let mut claims = sample_claims();
        claims.not_before = NOW + 60;
        claims.issued_at = IAT;
        let cwt = mint(&claims, &issuer, &ed25519_signer().public_key()).unwrap();
        assert!(matches!(
            verify(&cwt, NOW, &[issuer.public_key()]),
            Err(PermitError::NotYetValid { .. })
        ));
    }

    #[test]
    fn future_iat_rejected() {
        let issuer = ed25519_signer();
        let mut claims = sample_claims();
        claims.issued_at = NOW + 60;
        let cwt = mint(&claims, &issuer, &ed25519_signer().public_key()).unwrap();
        assert!(matches!(
            verify(&cwt, NOW, &[issuer.public_key()]),
            Err(PermitError::IssuedInFuture { .. })
        ));
    }

    #[test]
    fn signature_by_non_issuer_key_rejected() {
        // The header names a trusted issuer, so the kid lookup succeeds and
        // the failure surfaces at the signature check, not as
        // `UntrustedIssuer`.
        let issuer = ed25519_signer();
        let forger = ed25519_signer();
        let payload = encode(&claims_value(&sample_claims(), &forger.public_key()));
        let cwt = hand_built(
            payload,
            Algorithm::EdDSA,
            &issuer_kid(&issuer.public_key()),
            &forger,
        );
        assert!(matches!(
            verify(&cwt, NOW, &[issuer.public_key()]),
            Err(PermitError::InvalidSignature)
        ));
    }

    #[test]
    fn canonical_argument_hash_mismatch_rejected() {
        let issuer = ed25519_signer();
        let claims = sample_claims();
        let cwt = mint(&claims, &issuer, &ed25519_signer().public_key()).unwrap();
        let permit = verify(&cwt, NOW, &[issuer.public_key()]).unwrap();

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
        let issuer = ed25519_signer();
        let claims = sample_claims();
        let mut value = claims_value(&claims, &ed25519_signer().public_key());
        if let Value::Map(entries) = &mut value {
            entries.push((
                Value::Integer(ciborium::value::Integer::from(-79999)),
                Value::Text("future extension".to_string()),
            ));
        }
        let cwt = hand_built(
            encode(&value),
            Algorithm::EdDSA,
            &issuer_kid(&issuer.public_key()),
            &issuer,
        );
        let permit = verify(&cwt, NOW, &[issuer.public_key()]).unwrap();
        assert_eq!(permit.claims, claims);
    }

    #[test]
    fn parent_permit_chain_roundtrip() {
        let issuer = ed25519_signer();
        let parent_holder = ed25519_signer();
        let parent = mint(&sample_claims(), &issuer, &parent_holder.public_key()).unwrap();

        let child_holder = ed25519_signer();
        let mut child_claims = sample_claims();
        child_claims.tool_name = "arkavo.a2a.delegate".to_string();
        child_claims.parent_permit = Some(HashAlgorithm::Sha256.digest(&parent));
        let child = mint(&child_claims, &issuer, &child_holder.public_key()).unwrap();

        let permit = verify(&child, NOW, &[issuer.public_key()]).unwrap();
        // The parent hash binds the child to the exact parent CWT bytes.
        assert_eq!(
            permit.claims.parent_permit.as_deref(),
            Some(HashAlgorithm::Sha256.digest(&parent).as_slice())
        );
        assert_eq!(
            permit.confirmation_key.public_key_bytes(),
            child_holder.public_key().public_key_bytes()
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
            verify(&oversized, NOW, &[ed25519_signer().public_key()]),
            Err(PermitError::Cose(msg)) if msg.contains("maximum size")
        ));
    }

    #[test]
    fn decode_does_not_verify_but_validates_structure() {
        let issuer = ed25519_signer();
        let claims = sample_claims();
        let cwt = mint(&claims, &issuer, &ed25519_signer().public_key()).unwrap();
        // No trusted-issuer list and no signature check, but the structure
        // still has to hold.
        let permit = decode(&cwt).unwrap();
        assert_eq!(permit.claims, claims);
        assert!(decode(&cwt[..10]).is_err());
        assert!(decode(b"\xd8\x3dgarbage").is_err());
    }

    #[test]
    fn missing_budget_field_rejected() {
        let issuer = ed25519_signer();
        let claims = sample_claims();
        let mut value = claims_value(&claims, &ed25519_signer().public_key());
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
        let cwt = hand_built(
            encode(&value),
            Algorithm::EdDSA,
            &issuer_kid(&issuer.public_key()),
            &issuer,
        );
        assert!(matches!(
            verify(&cwt, NOW, &[issuer.public_key()]),
            Err(PermitError::MissingClaim("budget.max_invocations"))
        ));
    }

    #[test]
    fn cnf_key_recovers_expected_verifier() {
        let holder = ed25519_signer();
        let cwt = mint(&sample_claims(), &ed25519_signer(), &holder.public_key()).unwrap();
        let permit = decode(&cwt).unwrap();
        assert_eq!(
            permit.confirmation_key.public_key_bytes(),
            holder.public_key().public_key_bytes()
        );
    }

    #[test]
    fn id_is_stable_across_reencodings() {
        // One issuance, three byte strings. The permit's identity has to be
        // the same for all of them, or a holder re-encodes their way to a
        // fresh budget counter.
        let issuer = ed25519_signer();
        let holder = ed25519_signer();
        let cwt = mint(&sample_claims(), &issuer, &holder.public_key()).unwrap();
        let trusted = [issuer.public_key()];
        let original = verify(&cwt, NOW, &trusted).unwrap();
        let sign1 = parse_sign1(&cwt).unwrap().sign1;

        // Variant 1: the same COSE_Sign1 serialized bare (no tag 18), still
        // under the tag-61 prefix. The shared parser accepts both shapes.
        let mut bare = CWT_TAG_PREFIX.to_vec();
        bare.extend_from_slice(&sign1.clone().to_vec().unwrap());
        assert_ne!(bare, cwt, "dropping tag 18 must change the raw bytes");

        // Variant 2: an entry added to the unprotected header, which is
        // outside the signature and so leaves the issuer's signature valid.
        let mut padded_sign1 = sign1;
        padded_sign1
            .unprotected
            .rest
            .push((coset::Label::Int(-1000), Value::Text("padding".to_string())));
        let mut padded = CWT_TAG_PREFIX.to_vec();
        padded.extend_from_slice(&padded_sign1.to_tagged_vec().unwrap());
        assert_ne!(padded, cwt, "the extra header must change the raw bytes");

        // The padded variant never gets that far: verify refuses a non-empty
        // unprotected header outright.
        assert!(matches!(
            verify(&padded, NOW, &trusted),
            Err(PermitError::Cose(msg)) if msg.contains("unprotected header")
        ));

        // The bare variant verifies, and shares the original's identity.
        let reencoded = verify(&bare, NOW, &trusted).unwrap();
        assert_eq!(reencoded.id, original.id);
        assert_eq!(reencoded.claims, original.claims);
        // The unsigned header does not enter the digest either.
        assert_eq!(decode(&padded).unwrap().id, original.id);

        // A different issuance is a different identity.
        let mut other_claims = sample_claims();
        other_claims.tool_name = "arkavo.fs.write".to_string();
        let other = mint(&other_claims, &issuer, &holder.public_key()).unwrap();
        assert_ne!(verify(&other, NOW, &trusted).unwrap().id, original.id);
    }

    #[test]
    fn decode_reports_the_same_id_as_verify() {
        let issuer = ed25519_signer();
        let cwt = mint(&sample_claims(), &issuer, &ed25519_signer().public_key()).unwrap();
        assert_eq!(
            decode(&cwt).unwrap().id,
            verify(&cwt, NOW, &[issuer.public_key()]).unwrap().id
        );
    }

    #[test]
    fn id_covers_the_signed_structure_only() {
        // Sanity: the digest is SHA-256 over the COSE Sig_structure, not over
        // the wire bytes.
        let issuer = ed25519_signer();
        let cwt = mint(&sample_claims(), &issuer, &ed25519_signer().public_key()).unwrap();
        let permit = verify(&cwt, NOW, &[issuer.public_key()]).unwrap();
        let sign1 = parse_sign1(&cwt).unwrap().sign1;
        let expected: [u8; 32] = Sha256::digest(sign1.tbs_data(b"")).into();
        assert_eq!(permit.id, expected);
        assert_ne!(permit.id.as_slice(), Sha256::digest(&cwt).as_slice());
    }

    #[test]
    fn cbor_serializable_sign1_roundtrip_used_internally() {
        let cwt = mint(
            &sample_claims(),
            &ed25519_signer(),
            &ed25519_signer().public_key(),
        )
        .unwrap();
        let parsed = parse_sign1(&cwt).unwrap();
        let bytes = parsed.sign1.clone().to_vec().unwrap();
        let reparsed = CoseSign1::from_slice(&bytes).unwrap();
        assert_eq!(reparsed.signature, parsed.sign1.signature);
    }
}
