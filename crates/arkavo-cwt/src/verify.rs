//! COSE_Sign1 verification and CWT claim validation.

use crate::{Claims, CwtError, KeySet};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use coset::{CborSerializable, CoseSign1, RegisteredLabelWithPrivate};
use p256::ecdsa::Signature;
use p256::ecdsa::signature::Verifier;

/// authnz-rs prepends CBOR tag 61 (`cwt`) to an otherwise untagged COSE_Sign1.
const CWT_TAG: [u8; 2] = [0xD8, 0x3D];

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
/// Accepts the tag-61-prefixed wire form and a bare untagged COSE_Sign1.
/// ES256 is the only accepted algorithm.
pub fn verify(
    token_b64url: &str,
    keys: &KeySet,
    opts: &VerifyOptions<'_>,
) -> Result<Claims, CwtError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(token_b64url)
        .map_err(|e| CwtError::Base64(e.to_string()))?;
    let body = bytes.strip_prefix(&CWT_TAG[..]).unwrap_or(&bytes);
    let sign1 = CoseSign1::from_slice(body).map_err(|e| CwtError::Cose(e.to_string()))?;

    match &sign1.protected.header.alg {
        Some(RegisteredLabelWithPrivate::Assigned(coset::iana::Algorithm::ES256)) => {}
        Some(other) => return Err(CwtError::UnsupportedAlgorithm(format!("{other:?}"))),
        None => return Err(CwtError::UnsupportedAlgorithm("none".into())),
    }

    let kid = if sign1.protected.header.key_id.is_empty() {
        &sign1.unprotected.key_id
    } else {
        &sign1.protected.header.key_id
    };
    if kid.is_empty() {
        return Err(CwtError::MissingKid);
    }
    let key = keys
        .get(kid)
        .ok_or_else(|| CwtError::UnknownKid(hex(kid)))?;

    sign1.verify_signature(b"", |signature, signed| {
        let signature = Signature::from_slice(signature).map_err(|_| CwtError::BadSignature)?;
        key.verify(signed, &signature)
            .map_err(|_| CwtError::BadSignature)
    })?;

    let payload = sign1
        .payload
        .as_deref()
        .ok_or_else(|| CwtError::Cose("payload is detached".into()))?;
    let claims = Claims::from_cbor(payload)?;
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
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use crate::{CachedKeySet, CwtError, KeySet, VerifyOptions, verify};
    use arkavo_test_macros::spec;
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use ciborium::Value;
    use coset::{CborSerializable, CoseKeyBuilder, CoseKeySet, CoseSign1Builder, HeaderBuilder};
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
        let payload = Value::Map(vec![
            (Value::Integer(1.into()), Value::Text(ISS.into())),
            (
                Value::Integer(2.into()),
                Value::Text("did:key:zAgentUnderTest".into()),
            ),
            (
                Value::Integer(3.into()),
                Value::Array(aud.iter().map(|a| Value::Text((*a).into())).collect()),
            ),
            (Value::Integer(4.into()), Value::Integer(exp.into())),
            (Value::Integer(6.into()), Value::Integer((NOW - 60).into())),
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
            (
                Value::Text("act".into()),
                Value::Array(vec![Value::Map(vec![(
                    Value::Text("sub".into()),
                    Value::Text("did:key:zHumanPrincipal".into()),
                )])]),
            ),
        ]);
        let mut payload_bytes = Vec::new();
        ciborium::into_writer(&payload, &mut payload_bytes).expect("encode claims");

        let protected = HeaderBuilder::new()
            .algorithm(coset::iana::Algorithm::ES256)
            .key_id(kid.to_vec())
            .build();
        let sign1 = CoseSign1Builder::new()
            .protected(protected)
            .payload(payload_bytes)
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

    fn opts<'a>(aud: Option<&'a str>) -> VerifyOptions<'a> {
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
}
