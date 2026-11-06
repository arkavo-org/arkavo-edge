//! T25: fixtures this implementation did not produce, and an independent implementation
//! (the `httpsig` crate) verifying in both directions.

use crate::common::*;
use arkavo_tap::*;
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use httpsig::prelude::message_component::HttpMessageComponent;
use httpsig::prelude::{
    AlgorithmName, HttpSignatureBase, HttpSignatureHeaders, HttpSignatureParams, PublicKey,
    SecretKey,
};

/// RFC 9421 appendix B.1.4, test-key-ed25519.
const RFC_D: &str = "n4Ni-HpISpVObnQMW0wOhCKROaIKqKtW_2ZYb2p9KcU";
const RFC_X: &str = "JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs";
/// RFC 9421 appendix B.2.6.
const RFC_SIGNATURE: &str =
    "wqcAqbmYJ2ji2glfAMaRy4gruYYnx2nEFN2HN6jrnDnQCK1u02Gb04v9EDgwUPiu4A0w6vuQv5lIp5WPpBKRCw==";
const RFC_PARAMS: &str = r#"("date" "@method" "@path" "@authority" "content-type" "content-length");created=1618884473;keyid="test-key-ed25519""#;

fn rfc_request() -> RequestContext {
    RequestContext::new(
        ExternalOrigin::parse("https://example.com").unwrap(),
        "POST",
        "/foo?param=Value&Pet=dog",
        vec![
            ("Date".into(), "Tue, 20 Apr 2021 02:07:55 GMT".into()),
            ("Content-Type".into(), "application/json".into()),
            ("Content-Length".into(), "18".into()),
        ],
    )
    .unwrap()
}

fn rfc_record() -> KeyRecord {
    let x = URL_SAFE_NO_PAD.decode(RFC_X).unwrap();
    KeyRecord::ed25519(KeyFields {
        authority: authority(),
        key_id: "test-key-ed25519".into(),
        binding: ActorBinding {
            provider: "rfc9421".into(),
            runtime: None,
        },
        public_key: &x,
        status: KeyStatus::Active,
        not_before: 0,
        not_after: i64::MAX,
        purposes: vec![Purpose::Browse],
        retrieved_at: 0,
        refresh_by: i64::MAX,
        provenance: "RFC 9421 appendix B.1.4".into(),
    })
    .unwrap()
}

#[test]
fn rfc_9421_b_2_6_signature_is_reproduced_and_verified() {
    let components: Vec<String> = [
        "date",
        "@method",
        "@path",
        "@authority",
        "content-type",
        "content-length",
    ]
    .map(String::from)
    .to_vec();
    let base = arkavo_tap::base::signature_base(&rfc_request(), &components, RFC_PARAMS).unwrap();
    let expected = STANDARD.decode(RFC_SIGNATURE).unwrap();
    // Ed25519 is deterministic: the RFC's key over our base gives the RFC's signature.
    let d: [u8; 32] = URL_SAFE_NO_PAD.decode(RFC_D).unwrap().try_into().unwrap();
    assert_eq!(
        SigningKey::from_bytes(&d).sign(&base).to_bytes().to_vec(),
        expected
    );
    rfc_record().verify(&base, &expected).unwrap();
    // The RFC example lacks the parameters TAP requires, so the strict parser refuses it.
    let err = arkavo_tap::fields::parse(
        &format!("sig-b26={RFC_PARAMS}"),
        &format!("sig-b26=:{RFC_SIGNATURE}:"),
    )
    .unwrap_err();
    assert_eq!(err.kind, RejectKind::Malformed);
}

fn now() -> i64 {
    SystemClock.now()
}

/// Component lines built independently of `arkavo_tap`, from the raw request values.
fn lines(values: &[(&str, &str)]) -> Vec<HttpMessageComponent> {
    values
        .iter()
        .map(|(id, v)| HttpMessageComponent::try_from(format!("\"{id}\": {v}").as_str()).unwrap())
        .collect()
}

#[test]
fn httpsig_verifies_a_signature_made_here() {
    let (a, _) = agent(1, "agent-a", KeyStatus::Active);
    let body = br#"{"items":[{"id":"demo-001","quantity":1}]}"#;
    let digest = arkavo_tap::digest::content_digest(body);
    let ctx = write("/demo/v1/checkouts?ref=x", body, "idem-1");
    let op = arkavo_tap::signer::prepare(
        ctx,
        Profile::ArkavoRfc9421V1,
        Operation::Checkout,
        Purpose::Pay,
        "agent-a",
        now(),
        120,
    )
    .unwrap();
    let headers = arkavo_tap::signer::sign(&a, &op).unwrap();

    let parsed =
        HttpSignatureHeaders::try_parse(&headers.signature, &headers.signature_input).unwrap();
    let theirs = parsed.get("tap").unwrap();
    let base = HttpSignatureBase::try_new(
        &lines(&[
            ("@authority", "127.0.0.1:3001"),
            ("@path", "/demo/v1/checkouts"),
            ("@method", "POST"),
            (
                "@target-uri",
                "http://127.0.0.1:3001/demo/v1/checkouts?ref=x",
            ),
            ("content-type", "application/json"),
            ("content-digest", &digest),
            ("idempotency-key", "idem-1"),
        ]),
        theirs.signature_params(),
    )
    .unwrap();
    let pk = PublicKey::from_bytes(&AlgorithmName::Ed25519, &a.public_key()).unwrap();
    base.verify_signature_headers(&pk, theirs).unwrap();
    // A different method in their base must fail, so the check above is not vacuous.
    let wrong = HttpSignatureBase::try_new(
        &lines(&[
            ("@authority", "127.0.0.1:3001"),
            ("@path", "/demo/v1/checkouts"),
            ("@method", "PUT"),
            (
                "@target-uri",
                "http://127.0.0.1:3001/demo/v1/checkouts?ref=x",
            ),
            ("content-type", "application/json"),
            ("content-digest", &digest),
            ("idempotency-key", "idem-1"),
        ]),
        theirs.signature_params(),
    )
    .unwrap();
    assert!(wrong.verify_signature_headers(&pk, theirs).is_err());
}

#[tokio::test]
async fn a_signature_made_by_httpsig_verifies_here() {
    // httpsig checks expiry only when it verifies, so the fixed test time works here.
    let created = NOW;
    let (_a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(created);
    let v = verifier_at(
        &dir.path().join("r.sqlite"),
        StaticResolver::new(vec![rec]),
        clock,
        authority(),
    )
    .await;

    let nonce = arkavo_tap::signer::fresh_nonce();
    let params = HttpSignatureParams::try_from(
        format!(
            r#"("@authority" "@path" "@method" "@target-uri");created={created};expires={};nonce="{nonce}";alg="ed25519";keyid="agent-a";tag="agent-browser-auth""#,
            created + 120
        )
        .as_str(),
    )
    .unwrap();
    let base = HttpSignatureBase::try_new(
        &lines(&[
            ("@authority", "127.0.0.1:3001"),
            ("@path", "/demo/v1/products/demo-001"),
            ("@method", "GET"),
            (
                "@target-uri",
                "http://127.0.0.1:3001/demo/v1/products/demo-001",
            ),
        ]),
        &params,
    )
    .unwrap();
    let sk = SecretKey::from_bytes(&AlgorithmName::Ed25519, &[1u8; 32]).unwrap();
    let theirs = base.build_signature_headers(&sk, Some("tap")).unwrap();
    let ctx = with_fields(
        &request("GET", "/demo/v1/products/demo-001", &[]),
        &theirs.signature_input_header_value(),
        &theirs.signature_header_value(),
    );
    let agent = v
        .verify(
            &ctx,
            b"",
            &route(Operation::Read, Purpose::Browse),
            "httpsig",
        )
        .await
        .unwrap();
    assert_eq!(agent.key_id(), "agent-a");
}
