//! Purchase authority, idempotency, payment and receipt scenarios.

use super::*;

/// One fixture item ordered under a fresh grant.
pub(super) struct Placed {
    pub(super) answer: Answer,
    pub(super) checkout_id: String,
    pub(super) checkout_hash: String,
    pub(super) grant: SignedEnvelope,
}

pub(super) async fn order_one(env: &Env, key: &str) -> Result<Placed, String> {
    let grant = env.mint(|_| {}).map_err(|e| e.to_string())?;
    let (checkout_id, checkout_hash) =
        checkout(&env.a, &[("demo-001", 1)], &format!("{key}-checkout")).await?;
    let answer = got(env.a.order(&checkout_id, Some(&grant), key).await)?;
    expect(&answer, 201, None)?;
    Ok(Placed {
        answer,
        checkout_id,
        checkout_hash,
        grant,
    })
}

pub(super) fn receipt_payload(env: &Env, answer: &Answer) -> Result<serde_json::Value, String> {
    let envelope: SignedEnvelope =
        serde_json::from_value(answer.body["receipt"].clone()).map_err(|e| e.to_string())?;
    let bytes = arkavo_tap::envelope::open(&envelope, RECEIPT, &env.merchant_key)?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

pub(super) async fn denied(
    env: &Env,
    grant: Option<&SignedEnvelope>,
    items: &[(&str, u32)],
    key: &str,
) -> Check {
    let (checkout_id, _) = checkout(&env.a, items, &format!("{key}-checkout")).await?;
    let answer = got(env.a.order(&checkout_id, grant, key).await)?;
    expect(&answer, 403, Some("authority_denied"))
}

pub(super) fn provider_operation(env: &Env, answer: &Answer) -> Result<String, String> {
    Ok(str_of(
        &receipt_payload(env, answer)?["payment"],
        "providerOperation",
    )?
    .to_owned())
}

pub(super) async fn t17(env: &Env) -> Check {
    let (checkout_id, _) = checkout(&env.b, &[("demo-001", 1)], "t17-checkout").await?;
    let answer = got(env.b.order(&checkout_id, Some(&env.grant), "t17").await)?;
    expect(&answer, 403, Some("authority_denied"))
}

pub(super) async fn t18(env: &Env) -> Check {
    denied(env, None, &[("demo-001", 1)], "t18-absent").await?;
    let now = env.clock.now();
    let expired = env
        .mint(|g| g.expires_at = now)
        .map_err(|e| e.to_string())?;
    denied(env, Some(&expired), &[("demo-001", 1)], "t18-expired").await?;
    let revoked = env
        .mint(|g| g.grant_id = "t18-revoked".into())
        .map_err(|e| e.to_string())?;
    env.merchant
        .store
        .revoke_grant("t18-revoked")
        .await
        .map_err(|e| e.to_string())?;
    denied(env, Some(&revoked), &[("demo-001", 1)], "t18-revoked").await
}

pub(super) async fn t19(env: &Env) -> Check {
    let grant = env.mint(|_| {}).map_err(|e| e.to_string())?;
    denied(env, Some(&grant), &[("demo-003", 1)], "t19-item").await?;
    denied(env, Some(&grant), &[("demo-001", 3)], "t19-quantity").await?;
    let changes: [fn(&mut arkavo_tap::LocalTestGrant); 3] = [
        |g| g.merchant = "https://elsewhere.example".into(),
        |g| g.currency = "EUR".into(),
        |g| g.limit_minor = 100,
    ];
    for (i, change) in changes.into_iter().enumerate() {
        let grant = env.mint(change).map_err(|e| e.to_string())?;
        denied(env, Some(&grant), &[("demo-001", 1)], &format!("t19-{i}")).await?;
    }
    Ok(())
}

pub(super) async fn t20(env: &Env) -> Check {
    let first = order_one(env, "t20").await?;
    let again = got(env
        .a
        .order(&first.checkout_id, Some(&first.grant), "t20")
        .await)?;
    expect(&again, 201, None)?;
    ensure(
        again.body["orderId"] == first.answer.body["orderId"],
        "a retry must return the original order",
    )?;
    let op = provider_operation(env, &first.answer)?;
    let n = env.merchant.payments.operations(&op).await?;
    ensure(
        n == 1,
        format!("expected one provider operation, found {n}"),
    )
}

pub(super) async fn t21(env: &Env) -> Check {
    let grant = env.mint(|_| {}).map_err(|e| e.to_string())?;
    let (x, _) = checkout(&env.a, &[("demo-001", 1)], "t21-x").await?;
    let (y, _) = checkout(&env.a, &[("demo-002", 1)], "t21-y").await?;
    expect(&got(env.a.order(&x, Some(&grant), "t21").await)?, 201, None)?;
    expect(
        &got(env.a.order(&y, Some(&grant), "t21").await)?,
        409,
        Some("idempotency_conflict"),
    )
}

/// T22: the process stopped after the provider recorded the payment and before the order
/// result was stored. The retry must reconcile through the provider reference, not pay again.
pub(super) async fn t22(env: &Env) -> Check {
    let placed = order_one(env, "t22").await?;
    let order_id = str_of(&placed.answer.body, "orderId")?.to_owned();
    let op = provider_operation(env, &placed.answer)?;
    let pool = env.merchant.store.pool();
    for sql in [
        "UPDATE orders SET state = 'submitted', receipt = NULL WHERE order_id = ?",
        "UPDATE operations SET status = NULL, response = NULL WHERE order_id = ?",
    ] {
        sqlx::query(sql)
            .bind(&order_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    }
    let retried = got(env
        .a
        .order(&placed.checkout_id, Some(&placed.grant), "t22")
        .await)?;
    expect(&retried, 201, None)?;
    ensure(
        retried.body["orderId"] == order_id.as_str(),
        "reconciliation must keep the order",
    )?;
    let n = env.merchant.payments.operations(&op).await?;
    ensure(
        n == 1,
        format!("expected one provider operation, found {n}"),
    )?;
    let stored = env
        .merchant
        .store
        .order(&order_id)
        .await
        .map_err(|e| e.to_string())?;
    ensure(
        stored.is_some_and(|o| o.state == "simulated"),
        "the order must end simulated",
    )
}

pub(super) async fn t23(env: &Env) -> Check {
    let placed = order_one(env, "t23").await?;
    let payment = &placed.answer.body["payment"];
    ensure(
        payment["state"] == "simulated"
            && payment["simulated"] == true
            && payment["settlement"] == "none",
        format!("payment {payment}"),
    )?;
    let receipt = receipt_payload(env, &placed.answer)?;
    ensure(
        receipt["payment"]["state"] != "confirmed",
        "a simulation is never a confirmed payment",
    )
}

pub(super) async fn t24(env: &Env) -> Check {
    let production = env
        .mint(|g| g.environment = "production".into())
        .map_err(|e| e.to_string())?;
    denied(env, Some(&production), &[("demo-001", 1)], "t24").await
}

pub(super) async fn t26(env: &Env) -> Check {
    let evil = ExternalOrigin::parse("https://evil.example")?;
    let refused = env
        .a
        .prepare_for(&evil, &Draft::read("/steal", Purpose::Pay));
    ensure(
        matches!(refused, Err(SignError::Refused(_))),
        "the signer must refuse other destinations",
    )?;
    let (theirs, _) = checkout(&env.b, &[("demo-001", 1)], "t26-checkout").await?;
    let answer = got(env.a.order(&theirs, Some(&env.grant), "t26").await)?;
    expect(&answer, 404, Some("no_such_checkout"))
}

pub(super) async fn t28(env: &Env) -> Check {
    let refusal = got(env.stranger.browse("demo-001").await)?;
    let fields = refusal
        .body
        .as_object()
        .map(|o| o.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    ensure(
        fields == ["correlationId", "error"],
        format!("a refusal must name only its code and correlation ID: {fields:?}"),
    )?;
    let signer = Ed25519Signer::new(
        &env.agent_key.to_bytes(),
        "agent-a",
        SigningPolicy {
            destinations: vec![],
            operations: vec![],
        },
    );
    ensure(
        format!("{signer:?}").contains("<redacted>"),
        "signer debug output must redact the key",
    )
}

pub(super) async fn t29(env: &Env) -> Check {
    let placed = order_one(env, "t29").await?;
    let r = receipt_payload(env, &placed.answer)?;
    ensure(
        r["label"] == RECEIPT && r["environment"] == "local",
        format!("receipt {r}"),
    )?;
    ensure(
        r["actor"]["keyId"] == "agent-a" && r["actor"]["keyThumbprint"].is_string(),
        "receipt must identify the actor",
    )?;
    ensure(
        r["principal"] == "principal:local-demo-person" && r["authority"] == "local-test-grant",
        "receipt must name principal and authority",
    )?;
    ensure(
        r["checkoutId"] == placed.checkout_id.as_str()
            && r["checkoutHash"] == placed.checkout_hash.as_str(),
        "receipt must bind the checkout",
    )?;
    ensure(
        r["payment"]["simulated"] == true,
        "receipt must say the payment is simulated",
    )?;
    let order_id = str_of(&placed.answer.body, "orderId")?;
    expect(&got(env.a.order_status(order_id).await)?, 200, None)?;
    expect(
        &got(env.b.order_status(order_id).await)?,
        404,
        Some("no_such_order"),
    )
}

pub(super) async fn t30(env: &Env) -> Check {
    let anonymous = got(env.a.unsigned("GET", "/demo/v1/catalog", &[]).await)?;
    expect(&anonymous, 200, None)?;
    ensure(
        anonymous.body["verifiedAgent"] == false,
        "anonymous browsing is not verified",
    )?;
    let signed = got(env
        .a
        .send(&Draft::read("/demo/v1/catalog", Purpose::Browse))
        .await)?;
    ensure(
        signed.body["verifiedAgent"] == false,
        "an unprotected route grants no verified status",
    )
}
