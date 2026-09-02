//! COSE_Sign1 verification and CWT claim validation.

use crate::{Claims, CwtError, KeySet};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

/// What the caller expects the token to say, and when "now" is.
pub struct VerifyOptions<'a> {
    pub expected_iss: &'a str,
    /// `None` skips the audience check — appropriate where the caller does not
    /// know the issuer's configured audiences.
    pub expected_aud: Option<&'a str>,
    pub now: i64,
    pub skew_secs: i64,
}

/// Verify a base64url-encoded agent CWT and return its claims.
///
/// Accepts every envelope [`crate::sign1::parse`] does: the tag-61 prefix is
/// optional, and the COSE_Sign1 inside it may be tagged (tag 18) or bare.
/// ES256 is the only accepted algorithm, and decoded input larger than
/// [`crate::sign1::MAX_TOKEN_BYTES`] (16 KiB) is refused before parse.
pub fn verify(
    token_b64url: &str,
    keys: &KeySet,
    opts: &VerifyOptions<'_>,
) -> Result<Claims, CwtError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(token_b64url)
        .map_err(|e| CwtError::Base64(e.to_string()))?;
    let parsed = crate::sign1::parse(&bytes)?;
    if parsed.algorithm != coset::iana::Algorithm::ES256 {
        return Err(CwtError::UnsupportedAlgorithm(format!(
            "{:?}",
            parsed.algorithm
        )));
    }
    let kid = parsed.kid();
    if kid.is_empty() {
        return Err(CwtError::MissingKid);
    }
    let key = keys
        .get(kid)
        .ok_or_else(|| CwtError::UnknownKid(hex(kid)))?;
    parsed.verify(key)?;
    let claims = Claims::from_cbor(parsed.payload()?)?;
    check_claims(&claims, opts)?;
    Ok(claims)
}

fn check_claims(claims: &Claims, opts: &VerifyOptions<'_>) -> Result<(), CwtError> {
    if claims.iss != opts.expected_iss {
        return Err(CwtError::IssuerMismatch {
            expected: opts.expected_iss.to_string(),
            actual: claims.iss.clone(),
        });
    }
    if claims.exp.saturating_add(opts.skew_secs) < opts.now {
        return Err(CwtError::Expired {
            exp: claims.exp,
            now: opts.now,
        });
    }
    if claims.iat.saturating_sub(opts.skew_secs) > opts.now {
        return Err(CwtError::IssuedInFuture {
            iat: claims.iat,
            now: opts.now,
        });
    }
    if let Some(expected) = opts.expected_aud
        && !claims.aud.iter().any(|aud| aud == expected)
    {
        return Err(CwtError::AudienceMismatch {
            expected: expected.to_string(),
        });
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
            // Writing into a String cannot fail.
            let _ = write!(out, "{b:02x}");
            out
        })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use crate::{CachedKeySet, CwtError, KeySet, VerifyOptions, verify};
    use arkavo_test_macros::spec;
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use ciborium::Value;
    use coset::{
        CborSerializable, CoseKeyBuilder, CoseKeySet, CoseSign1, CoseSign1Builder, Header,
        HeaderBuilder,
    };
    use p256::ecdsa::{Signature, SigningKey, VerifyingKey, signature::Signer};
    use std::time::Duration;

    const ISS: &str = "https://identity.arkavo.net";
    const NOW: i64 = 1_800_000_000;

    fn new_key() -> SigningKey {
        SigningKey::random(&mut rand::rngs::OsRng)
    }

    /// Mint a token shaped exactly like an authnz-rs agent CWT: tag 61 wrapped
    /// around an untagged COSE_Sign1, ES256, raw-bytes `kid` in the protected
    /// header, integer claim keys plus the `arkavo_*` text claims.
    fn mint(signer: &SigningKey, kid: &[u8], aud: &[&str], exp: i64, tagged: bool) -> String {
        let aud = Value::Array(aud.iter().map(|a| Value::Text((*a).into())).collect());
        let act = Some(Value::Array(vec![Value::Map(vec![(
            Value::Text("sub".into()),
            Value::Text("did:key:zHumanPrincipal".into()),
        )])]));
        mint_claims(signer, kid, aud, exp, act, None, tagged)
    }

    /// Mint with full control over the shapes the review found untested:
    /// `aud` as either the agent shape (`Value::Array`) or the bare-text shape
    /// other token types use, `act` optionally omitted entirely (its documented
    /// absence when no actors are configured, not an empty array), and an
    /// arbitrary `arkavo_npe` payload.
    fn mint_claims(
        signer: &SigningKey,
        kid: &[u8],
        aud: Value,
        exp: i64,
        act: Option<Value>,
        npe: Option<Value>,
        tagged: bool,
    ) -> String {
        let payload = claims_cbor(ISS, aud, exp, NOW - 60, act, npe);
        encode_sign1(
            signer,
            es256_protected(kid),
            Header::default(),
            payload,
            tagged,
        )
    }

    /// The CBOR claims map an authnz-rs agent CWT carries: integer claim keys
    /// for the registered claims, literal text keys for the `arkavo_*` set.
    fn claims_cbor(
        iss: &str,
        aud: Value,
        exp: i64,
        iat: i64,
        act: Option<Value>,
        npe: Option<Value>,
    ) -> Vec<u8> {
        let mut fields = vec![
            (Value::Integer(1.into()), Value::Text(iss.into())),
            (
                Value::Integer(2.into()),
                Value::Text("did:key:zAgentUnderTest".into()),
            ),
            (Value::Integer(3.into()), aud),
            (Value::Integer(4.into()), Value::Integer(exp.into())),
            (Value::Integer(6.into()), Value::Integer(iat.into())),
            (
                Value::Text("arkavo_account_id".into()),
                Value::Text("acct-42".into()),
            ),
            (
                Value::Text("arkavo_roles".into()),
                Value::Array(vec![Value::Text("agent".into())]),
            ),
            (
                Value::Text("arkavo_entitlements".into()),
                Value::Array(vec![
                    Value::Text("https://arkavo.ai/attr/repo/read".into()),
                    Value::Text("https://arkavo.ai/attr/repo/write".into()),
                ]),
            ),
        ];
        if let Some(act) = act {
            fields.push((Value::Text("act".into()), act));
        }
        if let Some(npe) = npe {
            fields.push((Value::Text("arkavo_npe".into()), npe));
        }
        let mut bytes = Vec::new();
        ciborium::into_writer(&Value::Map(fields), &mut bytes).expect("encode claims");
        bytes
    }

    /// The protected header authnz-rs emits: ES256 plus the raw-bytes `kid`.
    fn es256_protected(kid: &[u8]) -> Header {
        HeaderBuilder::new()
            .algorithm(coset::iana::Algorithm::ES256)
            .key_id(kid.to_vec())
            .build()
    }

    /// Sign `payload` under the given headers and return the client-facing
    /// encoding: base64url-no-pad, optionally tag-61 prefixed.
    fn encode_sign1(
        signer: &SigningKey,
        protected: Header,
        unprotected: Header,
        payload: Vec<u8>,
        tagged: bool,
    ) -> String {
        let sign1 = CoseSign1Builder::new()
            .protected(protected)
            .unprotected(unprotected)
            .payload(payload)
            .create_signature(b"", |data| {
                let sig: Signature = signer.sign(data);
                sig.to_bytes().to_vec()
            })
            .build();

        let mut bytes = if tagged { vec![0xD8, 0x3D] } else { Vec::new() };
        bytes.extend_from_slice(&sign1.to_vec().expect("encode COSE_Sign1"));
        URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Encode a `/.well-known/cose-keys` body: a CBOR array of COSE_Key maps.
    fn key_set_cbor(entries: &[(&[u8], VerifyingKey)]) -> Vec<u8> {
        let keys = entries
            .iter()
            .map(|(kid, vk)| {
                let point = vk.to_encoded_point(false);
                CoseKeyBuilder::new_ec2_pub_key(
                    coset::iana::EllipticCurve::P_256,
                    point.x().expect("x coordinate").to_vec(),
                    point.y().expect("y coordinate").to_vec(),
                )
                .algorithm(coset::iana::Algorithm::ES256)
                .key_id(kid.to_vec())
                .build()
            })
            .collect();
        CoseKeySet(keys).to_vec().expect("encode key set")
    }

    fn opts(aud: Option<&str>) -> VerifyOptions<'_> {
        VerifyOptions {
            expected_iss: ISS,
            expected_aud: aud,
            now: NOW,
            skew_secs: 30,
        }
    }

    #[test]
    #[spec("ACWT-001")]
    fn verifies_es256_tagged_cwt_and_reads_arkavo_claims() {
        let signer = new_key();
        let kid = [0x11u8; 32];
        let keys = KeySet::from_cbor(&key_set_cbor(&[(&kid, *signer.verifying_key())]))
            .expect("parse key set");

        let token = mint(&signer, &kid, &["arkavo-kas"], NOW + 600, true);
        let claims = verify(&token, &keys, &opts(Some("arkavo-kas"))).expect("verify tagged CWT");

        assert_eq!(claims.iss, ISS);
        assert_eq!(claims.sub, "did:key:zAgentUnderTest");
        assert_eq!(claims.aud, vec!["arkavo-kas".to_string()]);
        assert_eq!(claims.exp, NOW + 600);
        assert_eq!(claims.iat, NOW - 60);
        assert_eq!(claims.account_id.as_deref(), Some("acct-42"));
        assert_eq!(claims.roles, vec!["agent".to_string()]);
        assert_eq!(
            claims.entitlements,
            vec![
                "https://arkavo.ai/attr/repo/read".to_string(),
                "https://arkavo.ai/attr/repo/write".to_string(),
            ]
        );
        assert_eq!(claims.actors, vec!["did:key:zHumanPrincipal".to_string()]);
        assert!(claims.npe.is_none());

        // The same bytes without the tag-61 prefix are equally acceptable.
        let untagged = mint(&signer, &kid, &["arkavo-kas"], NOW + 600, false);
        assert!(verify(&untagged, &keys, &opts(Some("arkavo-kas"))).is_ok());
    }

    #[test]
    #[spec("ACWT-002")]
    fn rejects_wrong_key_expired_and_wrong_audience() {
        let signer = new_key();
        let impostor = new_key();
        let kid = [0x22u8; 32];
        // The key set advertises `kid` but binds it to a different public key,
        // so a token minted by `impostor` reaches signature verification and
        // fails there rather than being rejected as an unknown kid.
        let wrong_key_set =
            KeySet::from_cbor(&key_set_cbor(&[(&kid, *signer.verifying_key())])).expect("key set");
        let token = mint(&impostor, &kid, &["arkavo-kas"], NOW + 600, true);
        assert!(matches!(
            verify(&token, &wrong_key_set, &opts(Some("arkavo-kas"))),
            Err(CwtError::BadSignature)
        ));

        let keys = KeySet::from_cbor(&key_set_cbor(&[(&kid, *impostor.verifying_key())]))
            .expect("key set");

        let expired = mint(&impostor, &kid, &["arkavo-kas"], NOW - 31, true);
        assert!(matches!(
            verify(&expired, &keys, &opts(Some("arkavo-kas"))),
            Err(CwtError::Expired { .. })
        ));

        let good = mint(&impostor, &kid, &["arkavo-kas"], NOW + 600, true);
        assert!(matches!(
            verify(&good, &keys, &opts(Some("some-other-service"))),
            Err(CwtError::AudienceMismatch { .. })
        ));
    }

    #[tokio::test]
    #[spec("ACWT-003")]
    async fn cached_keyset_refreshes_on_unknown_kid_once_per_ttl() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let signer = new_key();
        let stale = new_key();
        let kid = [0x33u8; 32];
        let token = mint(&signer, &kid, &["arkavo-kas"], NOW + 600, true);

        let server = MockServer::start().await;
        // First response predates the rotation and does not carry `kid`; every
        // later response does.
        Mock::given(method("GET"))
            .and(path("/.well-known/cose-keys"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(key_set_cbor(&[(&[0x99u8; 32], *stale.verifying_key())])),
            )
            .up_to_n_times(1)
            .expect(1..)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/.well-known/cose-keys"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(key_set_cbor(&[(&kid, *signer.verifying_key())])),
            )
            .mount(&server)
            .await;

        let url = format!("{}/.well-known/cose-keys", server.uri());

        // ttl 0: the unknown kid triggers an immediate re-fetch, which picks up
        // the rotated key and verifies.
        let eager = CachedKeySet::new(&url, Duration::ZERO);
        let claims = eager
            .verify(&token, &opts(Some("arkavo-kas")))
            .await
            .expect("verify after refresh");
        assert_eq!(claims.sub, "did:key:zAgentUnderTest");
        assert_eq!(server.received_requests().await.unwrap().len(), 2);

        // A long ttl suppresses the re-fetch: the unknown kid is reported and
        // the key set is fetched exactly once.
        let lazy = CachedKeySet::new(&url, Duration::from_secs(3600));
        let before = server.received_requests().await.unwrap().len();
        let stale_kid_token = mint(&stale, &[0x44u8; 32], &["arkavo-kas"], NOW + 600, true);
        assert!(matches!(
            lazy.verify(&stale_kid_token, &opts(Some("arkavo-kas")))
                .await,
            Err(CwtError::UnknownKid(_))
        ));
        assert!(
            lazy.verify(&stale_kid_token, &opts(Some("arkavo-kas")))
                .await
                .is_err()
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            before + 1,
            "the key set must be re-fetched at most once per ttl"
        );
    }

    /// authnz-contract.md: agent tokens carry `arkavo_npe: {type: "agent",
    /// delegation_id, depth, chain}` and omit `act` entirely when no actors
    /// are configured (not an empty array). Before this test the mint helper
    /// never emitted `arkavo_npe`, so no test drove `Claims::to_json` — the
    /// crate's largest function — through a real, signed agent-shaped token.
    #[test]
    fn accepts_agent_shaped_token_with_npe_and_no_act() {
        let signer = new_key();
        let kid = [0x55u8; 32];
        let keys = KeySet::from_cbor(&key_set_cbor(&[(&kid, *signer.verifying_key())]))
            .expect("parse key set");

        let npe = Value::Map(vec![
            (Value::Text("type".into()), Value::Text("agent".into())),
            (
                Value::Text("delegation_id".into()),
                Value::Text("did:key:zAgentUnderTest".into()),
            ),
            (Value::Text("depth".into()), Value::Integer(1.into())),
            (
                Value::Text("chain".into()),
                Value::Array(vec![
                    Value::Text("did:key:zRootPrincipal".into()),
                    Value::Text("did:key:zAgentUnderTest".into()),
                ]),
            ),
        ]);
        let aud = Value::Array(vec![Value::Text("arkavo-kas".into())]);
        let token = mint_claims(&signer, &kid, aud, NOW + 600, None, Some(npe), true);

        let claims = verify(&token, &keys, &opts(Some("arkavo-kas"))).expect("verify agent CWT");

        assert!(
            claims.actors.is_empty(),
            "act must be absent, not a bogus empty array: {:?}",
            claims.actors
        );
        assert_eq!(
            claims.npe,
            Some(serde_json::json!({
                "type": "agent",
                "delegation_id": "did:key:zAgentUnderTest",
                "depth": 1,
                "chain": ["did:key:zRootPrincipal", "did:key:zAgentUnderTest"],
            })),
            "arkavo_npe must survive into the JSON claims unchanged"
        );
    }

    /// authnz-contract.md: "Agent-token `aud` is ALWAYS a CBOR array ...
    /// Other token types use a bare text string — a verifier must accept
    /// both." This mints the non-agent, bare-text shape (the array shape is
    /// already exercised by every other test in this module).
    #[test]
    fn accepts_bare_text_audience_for_non_agent_token() {
        let signer = new_key();
        let kid = [0x66u8; 32];
        let keys = KeySet::from_cbor(&key_set_cbor(&[(&kid, *signer.verifying_key())]))
            .expect("parse key set");

        let aud = Value::Text("arkavo-kas".into());
        let token = mint_claims(&signer, &kid, aud, NOW + 600, None, None, true);

        let claims = verify(&token, &keys, &opts(Some("arkavo-kas")))
            .expect("verify token with bare-text aud");

        assert_eq!(claims.aud, vec!["arkavo-kas".to_string()]);
        assert!(claims.actors.is_empty());
        assert!(claims.npe.is_none());
    }

    /// agent-cwt.spec.yaml invariant: "ES256 only; any other COSE algorithm is
    /// refused before a key is even looked up." Nothing pinned that; a verifier
    /// that honoured the token's own `alg` would let an attacker pick a weaker
    /// one, or none at all.
    #[test]
    fn refuses_any_algorithm_other_than_es256() {
        let signer = new_key();
        let kid = [0x77u8; 32];
        let keys = KeySet::from_cbor(&key_set_cbor(&[(&kid, *signer.verifying_key())]))
            .expect("parse key set");
        let aud = Value::Array(vec![Value::Text("arkavo-kas".into())]);

        let eddsa = encode_sign1(
            &signer,
            HeaderBuilder::new()
                .algorithm(coset::iana::Algorithm::EdDSA)
                .key_id(kid.to_vec())
                .build(),
            Header::default(),
            claims_cbor(ISS, aud.clone(), NOW + 600, NOW - 60, None, None),
            true,
        );
        assert!(matches!(
            verify(&eddsa, &keys, &opts(Some("arkavo-kas"))),
            Err(CwtError::UnsupportedAlgorithm(_))
        ));

        let no_alg = encode_sign1(
            &signer,
            HeaderBuilder::new().key_id(kid.to_vec()).build(),
            Header::default(),
            claims_cbor(ISS, aud, NOW + 600, NOW - 60, None, None),
            true,
        );
        assert!(matches!(
            verify(&no_alg, &keys, &opts(Some("arkavo-kas"))),
            Err(CwtError::UnsupportedAlgorithm(_))
        ));
    }

    /// A correctly signed token minted by some other issuer must not be
    /// accepted just because its signature verifies under a key we publish.
    #[test]
    fn refuses_a_wrong_issuer() {
        let signer = new_key();
        let kid = [0x88u8; 32];
        let keys = KeySet::from_cbor(&key_set_cbor(&[(&kid, *signer.verifying_key())]))
            .expect("parse key set");

        let token = encode_sign1(
            &signer,
            es256_protected(&kid),
            Header::default(),
            claims_cbor(
                "https://identity.attacker.example",
                Value::Array(vec![Value::Text("arkavo-kas".into())]),
                NOW + 600,
                NOW - 60,
                None,
                None,
            ),
            true,
        );

        assert!(matches!(
            verify(&token, &keys, &opts(Some("arkavo-kas"))),
            Err(CwtError::IssuerMismatch { .. })
        ));
    }

    /// A token whose `iat` is beyond the allowed skew has not been issued yet;
    /// accepting it would let a clock-skewed or forged token extend its own
    /// window past the 15-minute agent CWT lifetime.
    #[test]
    fn refuses_a_token_issued_in_the_future() {
        let signer = new_key();
        let kid = [0x99u8; 32];
        let keys = KeySet::from_cbor(&key_set_cbor(&[(&kid, *signer.verifying_key())]))
            .expect("parse key set");

        let token = encode_sign1(
            &signer,
            es256_protected(&kid),
            Header::default(),
            claims_cbor(
                ISS,
                Value::Array(vec![Value::Text("arkavo-kas".into())]),
                NOW + 600,
                NOW + 31,
                None,
                None,
            ),
            true,
        );

        assert!(matches!(
            verify(&token, &keys, &opts(Some("arkavo-kas"))),
            Err(CwtError::IssuedInFuture { .. })
        ));
    }

    /// Flipping one byte of the signed payload must be caught. Without this the
    /// suite could pass with the signature check reduced to a no-op: every other
    /// rejection test fails on a header, a clock or a claim comparison.
    #[test]
    fn refuses_a_tampered_payload() {
        let signer = new_key();
        let kid = [0xAAu8; 32];
        let keys = KeySet::from_cbor(&key_set_cbor(&[(&kid, *signer.verifying_key())]))
            .expect("parse key set");

        let token = mint(&signer, &kid, &["arkavo-kas"], NOW + 600, true);
        assert!(verify(&token, &keys, &opts(Some("arkavo-kas"))).is_ok());

        let bytes = URL_SAFE_NO_PAD.decode(&token).expect("decode token");
        let body = bytes.strip_prefix(&[0xD8u8, 0x3D][..]).expect("tag 61");
        let mut sign1 = CoseSign1::from_slice(body).expect("parse COSE_Sign1");
        sign1.payload.as_mut().expect("attached payload")[0] ^= 0x01;
        let tampered = URL_SAFE_NO_PAD.encode(sign1.to_vec().expect("re-encode"));

        assert!(matches!(
            verify(&tampered, &keys, &opts(Some("arkavo-kas"))),
            Err(CwtError::BadSignature)
        ));
    }

    /// authnz-contract.md: "`kid`: in the **protected** header." An unprotected
    /// `kid` is not covered by the signature, so it is not accepted even as a
    /// lookup hint.
    #[test]
    fn refuses_a_kid_carried_only_in_the_unprotected_header() {
        let signer = new_key();
        let kid = [0xBBu8; 32];
        let keys = KeySet::from_cbor(&key_set_cbor(&[(&kid, *signer.verifying_key())]))
            .expect("parse key set");

        let token = encode_sign1(
            &signer,
            HeaderBuilder::new()
                .algorithm(coset::iana::Algorithm::ES256)
                .build(),
            HeaderBuilder::new().key_id(kid.to_vec()).build(),
            claims_cbor(
                ISS,
                Value::Array(vec![Value::Text("arkavo-kas".into())]),
                NOW + 600,
                NOW - 60,
                None,
                None,
            ),
            true,
        );

        assert!(matches!(
            verify(&token, &keys, &opts(Some("arkavo-kas"))),
            Err(CwtError::MissingKid)
        ));
    }
}
