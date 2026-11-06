//! Signature, coverage, time, replay and trust scenarios.

use super::*;

pub(super) async fn t01(env: &Env) -> Check {
    let answer = got(env.a.browse("demo-001").await)?;
    expect(&answer, 200, None)?;
    let actor = &answer.body["verifiedActor"];
    ensure(
        actor["keyId"] == "agent-a" && actor["runtimeIdentified"] == true,
        format!("actor {actor}"),
    )?;
    ensure(
        answer.body["principalResolved"] == false,
        "a signature must not resolve a principal",
    )?;
    ensure(
        answer.body["purchaseAuthority"] == "none",
        "a signature must not imply purchase authority",
    )
}

pub(super) async fn t02(env: &Env) -> Check {
    expect(
        &got(env
            .a
            .unsigned("GET", "/demo/v1/products/demo-001", &[])
            .await)?,
        401,
        Some("missing_proof"),
    )
}

pub(super) async fn t03(env: &Env) -> Check {
    let mut moved = env
        .a
        .prepare(&Draft::read("/demo/v1/products/demo-001", Purpose::Browse))
        .map_err(|e| e.to_string())?;
    moved.path = "/demo/v1/products/demo-002".into();
    expect(
        &got(env.a.dispatch(&moved).await)?,
        401,
        Some("invalid_signature"),
    )?;
    // Signed for another authority (the same server under another name).
    let port = env
        .config
        .origin()
        .map_err(|e| e.to_string())?
        .authority()
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_owned();
    let alias = ExternalOrigin::parse(&format!("http://localhost:{port}"))?;
    let signer = Ed25519Signer::new(
        &env.agent_key.to_bytes(),
        "agent-a",
        SigningPolicy {
            destinations: vec![alias.as_origin()],
            operations: vec![Operation::Read],
        },
    );
    let ctx = arkavo_tap::RequestContext::new(alias, "GET", "/demo/v1/products/demo-001", vec![])
        .map_err(|e| e.detail)?;
    let op = arkavo_tap::signer::prepare(
        ctx,
        arkavo_tap::Profile::ArkavoRfc9421V1,
        Operation::Read,
        Purpose::Browse,
        "agent-a",
        env.clock.now(),
        120,
    )
    .map_err(|e| e.to_string())?;
    let h = arkavo_tap::signer::sign(&signer, &op).map_err(|e| e.to_string())?;
    let elsewhere = Prepared {
        method: "GET".into(),
        path: "/demo/v1/products/demo-001".into(),
        headers: vec![
            ("signature-input".into(), h.signature_input),
            ("signature".into(), h.signature),
        ],
        body: Vec::new(),
    };
    expect(
        &got(env.a.dispatch(&elsewhere).await)?,
        401,
        Some("invalid_signature"),
    )
}

pub(super) async fn t04(env: &Env) -> Check {
    let read = Draft::read("/demo/v1/products/demo-001", Purpose::Browse);
    let mut head = env.a.prepare(&read).map_err(|e| e.to_string())?;
    head.method = "HEAD".into();
    ensure(
        got(env.a.dispatch(&head).await)?.status == 401,
        "a changed method must fail",
    )?;
    let mut query = env.a.prepare(&read).map_err(|e| e.to_string())?;
    query.path.push_str("?quantity=9");
    expect(
        &got(env.a.dispatch(&query).await)?,
        401,
        Some("invalid_signature"),
    )
}

pub(super) async fn t05(env: &Env) -> Check {
    let body = br#"{"items":[{"id":"demo-001","quantity":1}]}"#;
    let mut altered = env
        .a
        .prepare(&Draft::write(
            "/demo/v1/checkouts",
            body,
            Operation::Checkout,
            "t05",
        ))
        .map_err(|e| e.to_string())?;
    altered.body = br#"{"items":[{"id":"demo-001","quantity":2}]}"#.to_vec();
    expect(
        &got(env.a.dispatch(&altered).await)?,
        401,
        Some("invalid_signature"),
    )
}

pub(super) async fn t06(env: &Env) -> Check {
    let body = br#"{"items":[{"id":"demo-001","quantity":1}]}"#;
    let mut draft = Draft::write("/demo/v1/checkouts", body, Operation::Checkout, "t06");
    draft.operation = Operation::Read;
    expect(&got(env.a.send(&draft).await)?, 401, Some("missing_proof"))
}

pub(super) async fn t07(env: &Env) -> Check {
    let mut dup = env
        .a
        .prepare(&Draft::read("/demo/v1/products/demo-001", Purpose::Browse))
        .map_err(|e| e.to_string())?;
    for (name, value) in &mut dup.headers {
        if name == "signature-input" {
            value.push_str(";nonce=\"again\"");
        }
    }
    expect(&got(env.a.dispatch(&dup).await)?, 400, Some("malformed"))
}

pub(super) async fn t08(env: &Env) -> Check {
    expect(
        &got(env.stranger.browse("demo-001").await)?,
        403,
        Some("untrusted_actor"),
    )
}

pub(super) async fn t09(env: &Env) -> Check {
    let k = &env.config.keys;
    for skew in [5, -500] {
        let clock = Arc::new(ManualClock::new(env.clock.now() + skew));
        let skewed = Buyer::from_config(&env.config, &k.agent, &k.agent_key_id, clock)
            .map_err(|e| e.to_string())?;
        expect(&got(skewed.browse("demo-001").await)?, 401, Some("expired"))?;
    }
    Ok(())
}

pub(super) async fn t10(env: &Env) -> Check {
    let once = env
        .a
        .prepare(&Draft::read("/demo/v1/products/demo-002", Purpose::Browse))
        .map_err(|e| e.to_string())?;
    expect(&got(env.a.dispatch(&once).await)?, 200, None)?;
    expect(&got(env.a.dispatch(&once).await)?, 409, Some("replay"))
}

pub(super) async fn t15(env: &Env) -> Check {
    let forged = [("x-agent-verified", "true"), ("x-tap-key-id", "agent-a")];
    expect(
        &got(env
            .a
            .unsigned("GET", "/demo/v1/products/demo-001", &forged)
            .await)?,
        401,
        Some("missing_proof"),
    )?;
    let catalog = got(env.a.unsigned("GET", "/demo/v1/catalog", &forged).await)?;
    ensure(
        catalog.body["verifiedAgent"] == false,
        "forged headers must not grant verified status",
    )
}

/// T14: a revocation takes effect once the cached key record reaches its refresh deadline.
pub(super) async fn t14(env: &Env) -> Check {
    expect(&got(env.b.browse("demo-001").await)?, 200, None)?;
    env.revoke("agent-b").await.map_err(|e| e.to_string())?;
    env.clock.advance(env.config.trust.key_max_age_seconds + 1);
    expect(
        &got(env.b.browse("demo-001").await)?,
        403,
        Some("untrusted_actor"),
    )
}

/// T13: with the registry gone and the cached record stale, protected requests fail closed.
pub(super) async fn t13(env: &Env) -> Check {
    env.stop_registry().await.map_err(|e| e.to_string())?;
    env.clock.advance(env.config.trust.key_max_age_seconds + 1);
    expect(
        &got(env.a.browse("demo-001").await)?,
        503,
        Some("unavailable"),
    )
}
