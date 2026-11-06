//! Committing an order: verified actor, separate purchase authority, idempotent commitment,
//! payment through a stable provider operation reference, and a signed receipt.

use super::payment::{PaymentAdapter as _, PaymentState};
use super::store::{Operation as Op, OrderRecord};
use super::{Merchant, failure, fingerprint, respond};
use arkavo_tap::{AuthorizationDecision, PurchaseAuthorizer as _, SignedEnvelope, VerifiedAgent};
use arkavo_tap_http::CorrelationId;
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::Deserialize;
use std::sync::Arc;

/// The envelope type of a receipt. Signed by the merchant key; never a grant.
pub const RECEIPT: &str = "simulated-receipt";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OrderRequest {
    checkout_id: String,
    #[serde(default)]
    grant: Option<SignedEnvelope>,
}

pub(super) async fn place(
    State(merchant): State<Arc<Merchant>>,
    Extension(agent): Extension<VerifiedAgent>,
    Extension(id): Extension<CorrelationId>,
    body: Bytes,
) -> Response {
    let scope = merchant.scope(&agent, "commit-order");
    let result = match merchant.store.begin(&scope, &fingerprint(&body)).await {
        Ok(Op::Done { status, response }) => return respond((status, response)),
        Ok(Op::Conflict) => {
            return respond(failure(StatusCode::CONFLICT, "idempotency_conflict", &id));
        }
        Ok(Op::InProgress {
            order_id: Some(order_id),
        }) => settle(&merchant, &order_id, &id).await,
        Ok(Op::New | Op::InProgress { order_id: None }) => {
            commit(&merchant, &agent, &scope, &body, &id).await
        }
        Err(_) => return respond(failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", &id)),
    };
    // A dependency outage is not a business result: leave the key open for a retry.
    if result.0 != StatusCode::SERVICE_UNAVAILABLE.as_u16()
        && merchant
            .store
            .finish(&scope, result.0, &result.1)
            .await
            .is_err()
    {
        return respond(failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", &id));
    }
    respond(result)
}

async fn commit(
    merchant: &Merchant,
    agent: &VerifiedAgent,
    scope: &str,
    body: &[u8],
    id: &CorrelationId,
) -> (u16, String) {
    let Ok(request) = serde_json::from_slice::<OrderRequest>(body) else {
        return failure(StatusCode::BAD_REQUEST, "invalid_order", id);
    };
    let Some(grant) = request.grant else {
        return failure(StatusCode::FORBIDDEN, "authority_denied", id);
    };
    let checkout = match merchant.store.checkout(&request.checkout_id).await {
        Ok(Some((actor, view))) if actor == agent.key_thumbprint() => view,
        Ok(_) => return failure(StatusCode::NOT_FOUND, "no_such_checkout", id),
        Err(_) => return failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", id),
    };
    let decision = match merchant
        .authorizer
        .authorize(agent, &checkout, &grant, &merchant.store)
        .await
    {
        Ok(decision) => decision,
        Err(rejection) => {
            tracing::warn!(correlation_id = %id.0, reason = %rejection.detail, "purchase authority denied");
            let status =
                StatusCode::from_u16(rejection.kind.status()).unwrap_or(StatusCode::FORBIDDEN);
            return failure(status, rejection.kind.code(), id);
        }
    };
    let order = OrderRecord {
        order_id: uuid::Uuid::new_v4().to_string(),
        actor: agent.key_thumbprint().to_owned(),
        checkout_id: checkout.checkout_id.clone(),
        grant_id: decision.grant_id.clone(),
        total_minor: checkout.total_minor,
        currency: checkout.currency.clone(),
        state: PaymentState::Authorized.as_str().to_owned(),
        provider_operation: format!("sim-op-{}", uuid::Uuid::new_v4()),
        decision: receipt_decision(agent, &decision),
        receipt: None,
    };
    match merchant.store.reserve(&order, decision.limit_minor).await {
        Ok(true) => {}
        Ok(false) => return failure(StatusCode::FORBIDDEN, "authority_denied", id),
        Err(_) => return failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", id),
    }
    if merchant
        .store
        .attach_order(scope, &order.order_id)
        .await
        .is_err()
    {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", id);
    }
    settle(merchant, &order.order_id, id).await
}

/// The verified actor and authorization evidence, as the receipt states them.
fn receipt_decision(agent: &VerifiedAgent, decision: &AuthorizationDecision) -> String {
    serde_json::json!({
        "actor": {
            "keyId": agent.key_id(), "keyThumbprint": agent.key_thumbprint(),
            "provider": agent.provider(), "runtime": agent.runtime(),
            "trustAuthority": agent.authority().to_string(),
        },
        "authorization": decision,
    })
    .to_string()
}

/// Submits (or reconciles) the order's payment, then issues its receipt. Safe to run again
/// for the same order: the provider is idempotent on the operation reference.
pub(super) async fn settle(
    merchant: &Merchant,
    order_id: &str,
    id: &CorrelationId,
) -> (u16, String) {
    let Ok(Some(order)) = merchant.store.order(order_id).await else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", id);
    };
    if order.state == PaymentState::Authorized.as_str()
        && merchant
            .store
            .set_state(order_id, PaymentState::Submitted.as_str())
            .await
            .is_err()
    {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", id);
    }
    let outcome = match merchant.payments.lookup(&order.provider_operation).await {
        Ok(Some(known)) => Ok(known),
        Ok(None) => {
            merchant
                .payments
                .submit(
                    &order.provider_operation,
                    order.total_minor,
                    &order.currency,
                )
                .await
        }
        Err(e) => Err(e),
    };
    let Ok(outcome) = outcome else {
        let _ = merchant
            .store
            .set_state(order_id, PaymentState::OutcomeUnknown.as_str())
            .await;
        return failure(StatusCode::SERVICE_UNAVAILABLE, "outcome_unknown", id);
    };
    if merchant
        .store
        .set_state(order_id, outcome.state.as_str())
        .await
        .is_err()
    {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", id);
    }
    let receipt = receipt(merchant, &order, outcome.state, outcome.simulated);
    let receipt_json = serde_json::to_string(&receipt).unwrap_or_default();
    if merchant
        .store
        .set_receipt(order_id, &receipt_json)
        .await
        .is_err()
    {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", id);
    }
    let body = serde_json::json!({
        "orderId": order.order_id,
        "payment": { "state": outcome.state.as_str(), "simulated": outcome.simulated, "settlement": "none" },
        "receipt": receipt,
    });
    (StatusCode::CREATED.as_u16(), body.to_string())
}

fn receipt(
    merchant: &Merchant,
    order: &OrderRecord,
    state: PaymentState,
    simulated: bool,
) -> SignedEnvelope {
    let decision: serde_json::Value = serde_json::from_str(&order.decision).unwrap_or_default();
    let payload = serde_json::json!({
        "label": RECEIPT,
        "environment": "local",
        "orderId": order.order_id,
        "merchant": merchant.origin.as_origin(),
        "actor": decision["actor"],
        "principal": decision["authorization"]["principal"],
        "authority": decision["authorization"]["authority"],
        "grantId": order.grant_id,
        "checkoutId": order.checkout_id,
        "checkoutHash": decision["authorization"]["checkoutHash"],
        "verificationDecision": decision["authorization"]["verificationDecision"],
        "totalMinor": order.total_minor,
        "currency": order.currency,
        "payment": {
            "state": state.as_str(), "simulated": simulated,
            "providerOperation": order.provider_operation, "settlement": "none",
        },
        "issuedAt": merchant.clock.now(),
    });
    let bytes = serde_json::to_vec(&payload).unwrap_or_default();
    arkavo_tap::envelope::seal(RECEIPT, "merchant", &merchant.receipt_key, &bytes)
}

pub(super) async fn status(
    State(merchant): State<Arc<Merchant>>,
    Extension(agent): Extension<VerifiedAgent>,
    Extension(id): Extension<CorrelationId>,
    Path(order_id): Path<String>,
) -> Response {
    match merchant.store.order(&order_id).await {
        Ok(Some(order)) if order.actor == agent.key_thumbprint() => {
            let receipt: Option<serde_json::Value> = order
                .receipt
                .as_deref()
                .and_then(|r| serde_json::from_str(r).ok());
            respond((
                200,
                serde_json::json!({ "orderId": order.order_id, "state": order.state, "receipt": receipt })
                    .to_string(),
            ))
        }
        // Another actor's order is indistinguishable from no order.
        Ok(_) => respond(failure(StatusCode::NOT_FOUND, "no_such_order", &id)),
        Err(_) => respond(failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", &id)),
    }
}
