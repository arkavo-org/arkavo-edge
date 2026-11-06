use crate::common::*;
use arkavo_tap::*;
use ed25519_dalek::{Signer as _, SigningKey};

async fn setup(records: Vec<KeyRecord>) -> (tempfile::TempDir, Verifier, ManualClock) {
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(NOW);
    let v = verifier_at(
        &dir.path().join("replay.sqlite"),
        StaticResolver::new(records),
        clock.clone(),
        authority(),
    )
    .await;
    (dir, v, clock)
}

fn read_route() -> RoutePolicy {
    route(Operation::Read, Purpose::Browse)
}

async fn kind(v: &Verifier, ctx: &RequestContext, body: &[u8], r: &RoutePolicy) -> RejectKind {
    v.verify(ctx, body, r, "d").await.unwrap_err().kind
}

#[tokio::test]
async fn t01_a_valid_read_identifies_the_actor_and_nothing_more() {
    let (a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let thumb = rec.thumbprint();
    let (_d, v, _c) = setup(vec![rec]).await;
    let ctx = sign(
        &a,
        &request("GET", "/demo/v1/products/demo-001", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW,
    );
    let agent = v
        .verify(&ctx, b"", &read_route(), "decision-1")
        .await
        .unwrap();
    assert_eq!(agent.key_id(), "agent-a");
    assert_eq!(agent.key_thumbprint(), thumb);
    assert_eq!(agent.provider(), "arkavo");
    assert_eq!(agent.runtime(), Some("runtime-1"));
    assert_eq!(agent.authority(), &authority());
    assert_eq!(agent.profile(), Profile::ArkavoRfc9421V1);
    assert_eq!(agent.operation(), Operation::Read);
    assert_eq!(agent.purpose(), Purpose::Browse);
    assert_eq!(agent.decision_id(), "decision-1");
    assert_eq!(agent.provenance(), "static test registry");
    assert_eq!(agent.expires() - agent.created(), 120);
    assert_eq!(agent.nonce().len(), 43);
    assert_eq!(
        agent.covered_components(),
        ["@authority", "@path", "@method", "@target-uri"]
    );
    let b = agent.binding();
    assert_eq!(b.method, "GET");
    assert_eq!(
        b.target_uri,
        "http://127.0.0.1:3001/demo/v1/products/demo-001"
    );
    assert_eq!(b.content_digest, None);
    assert_eq!(b.idempotency_key, None);
}

#[tokio::test]
async fn t02_a_protected_route_without_a_signature_is_refused() {
    let (_a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let (_d, v, _c) = setup(vec![rec]).await;
    let ctx = request("GET", "/demo/v1/products/demo-001", &[]);
    assert_eq!(
        kind(&v, &ctx, b"", &read_route()).await,
        RejectKind::MissingProof
    );
    let half = request("GET", "/", &[("signature", "tap=:AQID:")]);
    assert_eq!(
        kind(&v, &half, b"", &read_route()).await,
        RejectKind::Malformed
    );
}

#[tokio::test]
async fn t03_t04_modified_authority_path_method_or_query_fail() {
    let (a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let (_d, v, _c) = setup(vec![rec]).await;
    let signed = sign(
        &a,
        &request("GET", "/demo/v1/products/demo-001?x=1", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW,
    );
    let input = signed.header("signature-input").unwrap();
    let sig = signed.header("signature").unwrap();
    for (method, target) in [
        ("GET", "/demo/v1/products/demo-002?x=1"),
        ("HEAD", "/demo/v1/products/demo-001?x=1"),
        ("GET", "/demo/v1/products/demo-001?x=2"),
        ("GET", "/demo/v1/products/demo-001"),
    ] {
        let tampered = with_fields(&request(method, target, &[]), &input, &sig);
        assert_eq!(
            kind(&v, &tampered, b"", &read_route()).await,
            RejectKind::InvalidSignature,
            "{method} {target}"
        );
    }
    // Signed for another merchant: the authority differs from this audience.
    let elsewhere = RequestContext::new(
        ExternalOrigin::parse("https://example.com").unwrap(),
        "GET",
        "/demo/v1/products/demo-001",
        vec![],
    )
    .unwrap();
    let op = arkavo_tap::signer::prepare(
        elsewhere.clone(),
        Profile::ArkavoRfc9421V1,
        Operation::Read,
        Purpose::Browse,
        "agent-a",
        NOW,
        120,
    )
    .unwrap();
    let h = arkavo_tap::signer::sign(&a, &op).unwrap();
    let replayed_here = with_fields(
        &request("GET", "/demo/v1/products/demo-001", &[]),
        &h.signature_input,
        &h.signature,
    );
    assert_eq!(
        kind(&v, &replayed_here, b"", &read_route()).await,
        RejectKind::InvalidSignature
    );
    // A context for another audience is refused before any lookup.
    let other = with_signature(&elsewhere, &h);
    assert_eq!(
        kind(&v, &other, b"", &read_route()).await,
        RejectKind::InvalidSignature
    );
}

#[tokio::test]
async fn t05_t06_content_must_match_a_covered_digest() {
    let (a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let (_d, v, _c) = setup(vec![rec]).await;
    let body = br#"{"items":[{"id":"demo-001","quantity":1}]}"#;
    let ctx = write("/demo/v1/checkouts", body, "idem-1");
    let signed = sign(&a, &ctx, Operation::Checkout, Purpose::Pay, NOW);
    let checkout = route(Operation::Checkout, Purpose::Pay);
    // T05: other content under the original digest.
    let altered = br#"{"items":[{"id":"demo-001","quantity":9}]}"#;
    assert_eq!(
        kind(&v, &signed, altered, &checkout).await,
        RejectKind::InvalidSignature
    );
    // T06: a valid digest that the signature does not cover.
    let read_coverage = sign(&a, &ctx, Operation::Read, Purpose::Pay, NOW);
    assert_eq!(
        kind(&v, &read_coverage, body, &checkout).await,
        RejectKind::MissingProof
    );
    // The intact request verifies and binds the digest and idempotency key.
    let agent = v.verify(&signed, body, &checkout, "d").await.unwrap();
    assert_eq!(agent.binding().idempotency_key.as_deref(), Some("idem-1"));
    assert!(
        agent
            .binding()
            .content_digest
            .as_deref()
            .unwrap()
            .starts_with("sha-256=:")
    );
}

#[tokio::test]
async fn t07_malformed_and_ambiguous_fields_are_refused_deterministically() {
    let (a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let (_d, v, _c) = setup(vec![rec]).await;
    let signed = sign(
        &a,
        &request("GET", "/p", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW,
    );
    let input = signed.header("signature-input").unwrap();
    let sig = signed.header("signature").unwrap();
    let cases = [
        (format!("{input};nonce=\"again\""), sig.clone()),
        (format!("{input}, tap2=(\"@path\");created=1"), sig.clone()),
        (input.replacen("tap=", "other=", 1), sig.clone()),
        (input.clone(), format!("{sig}, x=:AQID:")),
    ];
    for (i, s) in cases {
        let ctx = with_fields(&request("GET", "/p", &[]), &i, &s);
        for _ in 0..2 {
            assert_eq!(
                kind(&v, &ctx, b"", &read_route()).await,
                RejectKind::Malformed,
                "{i} {s}"
            );
        }
    }
}

#[tokio::test]
async fn t08_unknown_keys_and_wrong_algorithms_are_untrusted() {
    let (a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let (b, _not_registered) = agent(2, "agent-b", KeyStatus::Active);
    let (_d, v, _c) = setup(vec![rec]).await;
    let ctx = sign(
        &b,
        &request("GET", "/p", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW,
    );
    assert_eq!(
        kind(&v, &ctx, b"", &read_route()).await,
        RejectKind::UntrustedActor
    );
    // The Visa spelling of the algorithm is not accepted under the Arkavo profile.
    let op = arkavo_tap::signer::prepare(
        request("GET", "/p", &[]),
        Profile::VisaTapPinned,
        Operation::Read,
        Purpose::Browse,
        "agent-a",
        NOW,
        120,
    )
    .unwrap();
    let h = arkavo_tap::signer::sign(&a, &op).unwrap();
    let visa_alg = with_signature(&request("GET", "/p", &[]), &h);
    assert_eq!(
        kind(&v, &visa_alg, b"", &read_route()).await,
        RejectKind::MissingProof
    );
    // A key ID that could not be a registry key is refused before any lookup.
    let bad_id = custom(
        &a,
        "../agent-a",
        "ed25519",
        NOW,
        NOW + 60,
        "agent-browser-auth",
    );
    assert_eq!(
        kind(&v, &bad_id, b"", &read_route()).await,
        RejectKind::Malformed
    );
    let rsa = custom(
        &a,
        "agent-a",
        "rsa-pss-sha512",
        NOW,
        NOW + 60,
        "agent-browser-auth",
    );
    assert_eq!(
        kind(&v, &rsa, b"", &read_route()).await,
        RejectKind::UntrustedActor
    );
}

/// A read signed with hand-chosen parameters, for cases `prepare` refuses to build.
fn custom(
    signer: &Ed25519Signer,
    keyid: &str,
    alg: &str,
    created: i64,
    expires: i64,
    tag: &str,
) -> RequestContext {
    let ctx = request("GET", "/p", &[]);
    let params = arkavo_tap::fields::SignatureParams {
        created,
        expires,
        keyid: keyid.into(),
        alg: alg.into(),
        nonce: arkavo_tap::signer::fresh_nonce(),
        tag: tag.into(),
    };
    let comps = ["@authority", "@path", "@method", "@target-uri"];
    let (input, sp) = arkavo_tap::fields::signature_input("tap", &comps, &params).unwrap();
    let owned: Vec<String> = comps.iter().map(|c| (*c).into()).collect();
    let base = arkavo_tap::base::signature_base(&ctx, &owned, &sp).unwrap();
    let seed = if signer.key_id() == "agent-a" { 1 } else { 2 };
    let sig = SigningKey::from_bytes(&[seed; 32]).sign(&base).to_bytes();
    with_fields(
        &ctx,
        &input,
        &arkavo_tap::fields::signature("tap", &sig).unwrap(),
    )
}

#[tokio::test]
async fn t09_time_bounds_at_their_edges() {
    let (a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let (_d, v, clock) = setup(vec![rec]).await;
    let tag = "agent-browser-auth";
    // Lifetime: 480 seconds is the most any profile accepts.
    let ok = custom(&a, "agent-a", "ed25519", NOW, NOW + 480, tag);
    v.verify(&ok, b"", &read_route(), "d").await.unwrap();
    let long = custom(&a, "agent-a", "ed25519", NOW, NOW + 481, tag);
    assert_eq!(
        kind(&v, &long, b"", &read_route()).await,
        RejectKind::Expired
    );
    // Created in the future, by one second.
    let future = custom(&a, "agent-a", "ed25519", NOW + 1, NOW + 60, tag);
    assert_eq!(
        kind(&v, &future, b"", &read_route()).await,
        RejectKind::Expired
    );
    let backwards = custom(&a, "agent-a", "ed25519", NOW, NOW, tag);
    assert_eq!(
        kind(&v, &backwards, b"", &read_route()).await,
        RejectKind::Expired
    );
    // Valid until the second before `expires`.
    let edge = custom(&a, "agent-a", "ed25519", NOW, NOW + 60, tag);
    clock.set(NOW + 59);
    v.verify(&edge, b"", &read_route(), "d").await.unwrap();
    let edge2 = custom(&a, "agent-a", "ed25519", NOW, NOW + 60, tag);
    clock.set(NOW + 60);
    assert_eq!(
        kind(&v, &edge2, b"", &read_route()).await,
        RejectKind::Expired
    );
}

#[tokio::test]
async fn t10_t12_a_request_is_admitted_once_even_across_a_restart() {
    let (a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replay.sqlite");
    let clock = ManualClock::new(NOW);
    let v = verifier_at(
        &path,
        StaticResolver::new(vec![rec.clone()]),
        clock.clone(),
        authority(),
    )
    .await;
    let ctx = sign(
        &a,
        &request("GET", "/p", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW,
    );
    v.verify(&ctx, b"", &read_route(), "first").await.unwrap();
    assert_eq!(kind(&v, &ctx, b"", &read_route()).await, RejectKind::Replay);
    drop(v);
    let restarted = verifier_at(&path, StaticResolver::new(vec![rec]), clock, authority()).await;
    assert_eq!(
        kind(&restarted, &ctx, b"", &read_route()).await,
        RejectKind::Replay
    );
}

#[tokio::test]
async fn t11_two_verifier_instances_admit_a_request_at_most_once() {
    let (a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replay.sqlite");
    let clock = ManualClock::new(NOW);
    let v1 = verifier_at(
        &path,
        StaticResolver::new(vec![rec.clone()]),
        clock.clone(),
        authority(),
    )
    .await;
    let v2 = verifier_at(&path, StaticResolver::new(vec![rec]), clock, authority()).await;
    let ctx = sign(
        &a,
        &request("GET", "/p", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW,
    );
    let r = read_route();
    let (x, y) = tokio::join!(v1.verify(&ctx, b"", &r, "1"), v2.verify(&ctx, b"", &r, "2"));
    assert_eq!([x.is_ok(), y.is_ok()].iter().filter(|ok| **ok).count(), 1);
}

#[tokio::test]
async fn t13_unavailable_trust_or_replay_fails_closed() {
    let (a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let dir = tempfile::tempdir().unwrap();
    let clock = ManualClock::new(NOW);
    let mut down = StaticResolver::new(vec![rec.clone()]);
    down.unavailable = true;
    let v = verifier_at(
        &dir.path().join("a.sqlite"),
        down,
        clock.clone(),
        authority(),
    )
    .await;
    let ctx = sign(
        &a,
        &request("GET", "/p", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW,
    );
    assert_eq!(
        kind(&v, &ctx, b"", &read_route()).await,
        RejectKind::Unavailable
    );

    let store = SqliteReplayStore::open(&dir.path().join("b.sqlite"), clock.clone(), 30)
        .await
        .unwrap();
    store.close().await;
    let closed = TapVerifier::new(
        VerifierConfig {
            authority: authority(),
            audience: origin(),
            max_header_bytes: 16 * 1024,
        },
        StaticResolver::new(vec![rec]),
        store,
        clock,
    );
    assert_eq!(
        kind(&closed, &ctx, b"", &read_route()).await,
        RejectKind::Unavailable
    );
}

#[tokio::test]
async fn t14_t24_revoked_keys_and_other_environments_are_untrusted() {
    let (a, revoked) = agent(1, "agent-a", KeyStatus::Revoked);
    let (_d, v, _c) = setup(vec![revoked]).await;
    let ctx = sign(
        &a,
        &request("GET", "/p", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW,
    );
    assert_eq!(
        kind(&v, &ctx, b"", &read_route()).await,
        RejectKind::UntrustedActor
    );

    // A production verifier never consults the local development root.
    let (b, local) = agent(2, "agent-b", KeyStatus::Active);
    let dir = tempfile::tempdir().unwrap();
    let production = TrustAuthority {
        name: "arkavo-local-demo".into(),
        environment: Environment::Production,
    };
    let v = verifier_at(
        &dir.path().join("r.sqlite"),
        StaticResolver::new(vec![local]),
        ManualClock::new(NOW),
        production,
    )
    .await;
    let ctx = sign(
        &b,
        &request("GET", "/p", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW,
    );
    assert_eq!(
        kind(&v, &ctx, b"", &read_route()).await,
        RejectKind::UntrustedActor
    );
}

#[tokio::test]
async fn keys_outside_validity_or_purpose_and_routes_outside_purpose_are_refused() {
    let (a, mut rec) = agent(1, "agent-a", KeyStatus::Active);
    rec.purposes = vec![Purpose::Browse];
    let (_d, v, clock) = setup(vec![rec]).await;
    let pay = sign(
        &a,
        &request("GET", "/p", &[]),
        Operation::Read,
        Purpose::Pay,
        NOW,
    );
    let pay_route = route(Operation::Read, Purpose::Pay);
    assert_eq!(
        kind(&v, &pay, b"", &pay_route).await,
        RejectKind::PurposeDenied
    );
    let browse = sign(
        &a,
        &request("GET", "/p", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW,
    );
    assert_eq!(
        kind(&v, &browse, b"", &pay_route).await,
        RejectKind::PurposeDenied
    );
    let unknown_tag = custom(&a, "agent-a", "ed25519", NOW, NOW + 60, "agent-checkout");
    assert_eq!(
        kind(&v, &unknown_tag, b"", &read_route()).await,
        RejectKind::PurposeDenied
    );
    clock.set(NOW + 86_400);
    let late = sign(
        &a,
        &request("GET", "/p", &[]),
        Operation::Read,
        Purpose::Browse,
        NOW + 86_400,
    );
    assert_eq!(
        kind(&v, &late, b"", &read_route()).await,
        RejectKind::UntrustedActor
    );
}

#[tokio::test]
async fn oversized_headers_and_short_nonces_are_refused() {
    let (_a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let (_d, v, _c) = setup(vec![rec]).await;
    let big = "x".repeat(17 * 1024);
    let ctx = request("GET", "/p", &[("x-big", &big)]);
    assert_eq!(
        kind(&v, &ctx, b"", &read_route()).await,
        RejectKind::HeadersTooLarge
    );
    let ctx = request("GET", "/p", &[]);
    let params = arkavo_tap::fields::SignatureParams {
        created: NOW,
        expires: NOW + 60,
        keyid: "agent-a".into(),
        alg: "ed25519".into(),
        nonce: "short".into(),
        tag: "agent-browser-auth".into(),
    };
    let comps = ["@authority", "@path", "@method", "@target-uri"];
    let (input, _) = arkavo_tap::fields::signature_input("tap", &comps, &params).unwrap();
    let short = with_fields(&ctx, &input, "tap=:AQID:");
    assert_eq!(
        kind(&v, &short, b"", &read_route()).await,
        RejectKind::Malformed
    );
}

#[tokio::test]
async fn the_visa_profile_accepts_its_recognition_signature_for_reads_only() {
    let (_a, rec) = agent(1, "agent-a", KeyStatus::Active);
    let (_d, v, _c) = setup(vec![rec]).await;
    let visa_read = RoutePolicy {
        profile: Profile::VisaTapPinned,
        operation: Operation::Read,
        purposes: vec![Purpose::Browse],
    };
    // The parameter order of the Visa examples: created, keyid, alg, expires, nonce, tag.
    let ctx = request("GET", "/p?q=1", &[]);
    let nonce = "e8N7S2MFd/qrd6T2R3tdfAuuANngKI7LFtKYI/vowzk=";
    let sp = format!(
        r#"("@authority" "@path");created={NOW};keyid="agent-a";alg="Ed25519";expires={};nonce="{nonce}";tag="agent-browser-auth""#,
        NOW + 480
    );
    let base = arkavo_tap::base::signature_base(&ctx, &["@authority".into(), "@path".into()], &sp)
        .unwrap();
    let sig = SigningKey::from_bytes(&[1; 32]).sign(&base).to_bytes();
    let signed = with_fields(
        &ctx,
        &format!("sig2={sp}"),
        &arkavo_tap::fields::signature("sig2", &sig).unwrap(),
    );
    let agent = v.verify(&signed, b"", &visa_read, "d").await.unwrap();
    assert_eq!(agent.profile(), Profile::VisaTapPinned);
    // The baseline binds neither method nor content, so a Visa-profile write needs more.
    let visa_write = RoutePolicy {
        profile: Profile::VisaTapPinned,
        operation: Operation::Checkout,
        purposes: vec![Purpose::Browse],
    };
    let again = with_fields(
        &request("POST", "/p?q=1", &[]),
        &format!("sig2={sp}"),
        &arkavo_tap::fields::signature("sig2", &sig).unwrap(),
    );
    assert_eq!(
        kind(&v, &again, b"", &visa_write).await,
        RejectKind::MissingProof
    );
}
