//! The merchant fixture: a made-up catalog behind TAP verification, with immutable
//! checkouts, idempotent orders that need separate purchase authority, and simulated payment.
//!
//! Every product is development fixture data. These routes are private test contracts, not
//! UCP or Visa endpoints.

mod orders;
pub mod payment;
pub mod store;

pub use orders::RECEIPT;

use arkavo_tap::{
    CheckoutLine, CheckoutView, Clock, ExternalOrigin, LocalGrantAuthorizer, Operation, Profile,
    Purpose, ReplayStore, RoutePolicy, TrustedKeyResolver, VerifiedAgent,
};
use arkavo_tap_http::{CorrelationId, ProtectedRoute, TapGate, guard};
use axum::body::Bytes;
use axum::extract::{Extension, Path, State};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ed25519_dalek::{SigningKey, VerifyingKey};
use payment::SimulatedProvider;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use store::{Operation as Op, Store};

/// A fixture product.
#[derive(Debug, Clone, Copy)]
pub struct Product {
    pub id: &'static str,
    pub title: &'static str,
    pub unit_minor: i64,
    pub currency: &'static str,
}

/// The fixture catalog. Made-up items with made-up prices.
pub const CATALOG: [Product; 3] = [
    Product {
        id: "demo-001",
        title: "Fixture item one",
        unit_minor: 1_250,
        currency: "USD",
    },
    Product {
        id: "demo-002",
        title: "Fixture item two",
        unit_minor: 800,
        currency: "USD",
    },
    Product {
        id: "demo-003",
        title: "Fixture item three",
        unit_minor: 4_000,
        currency: "USD",
    },
];

fn product_json(p: &Product) -> serde_json::Value {
    serde_json::json!({
        "id": p.id, "title": p.title, "unitMinor": p.unit_minor,
        "currency": p.currency, "fixture": true,
    })
}

/// The merchant's state.
pub struct Merchant {
    pub store: Store,
    pub payments: SimulatedProvider,
    authorizer: LocalGrantAuthorizer<Arc<dyn Clock>>,
    receipt_key: SigningKey,
    origin: ExternalOrigin,
    clock: Arc<dyn Clock>,
}

impl Merchant {
    pub async fn open(
        database: &std::path::Path,
        origin: ExternalOrigin,
        issuers: HashMap<String, VerifyingKey>,
        receipt_key: SigningKey,
        clock: Arc<dyn Clock>,
    ) -> anyhow::Result<Self> {
        let store = Store::open(database).await?;
        let payments = SimulatedProvider::new(store.pool().clone());
        let authorizer =
            LocalGrantAuthorizer::new(issuers, arkavo_tap::Environment::Local, clock.clone());
        Ok(Merchant {
            store,
            payments,
            authorizer,
            receipt_key,
            origin,
            clock,
        })
    }

    /// The idempotency scope: actor, merchant, operation and key, hashed.
    fn scope(&self, agent: &VerifiedAgent, operation: &str) -> String {
        let key = agent
            .binding()
            .idempotency_key
            .as_deref()
            .unwrap_or_default();
        let mut h = Sha256::new();
        for part in [
            agent.key_thumbprint(),
            &self.origin.as_origin(),
            operation,
            key,
        ] {
            h.update((part.len() as u64).to_be_bytes());
            h.update(part.as_bytes());
        }
        hex(&h.finalize())
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

pub(crate) fn fingerprint(body: &[u8]) -> String {
    hex(&Sha256::digest(body))
}

pub(crate) fn failure(status: StatusCode, code: &str, id: &CorrelationId) -> (u16, String) {
    let body = serde_json::json!({ "error": code, "correlationId": id.0 });
    (status.as_u16(), body.to_string())
}

pub(crate) fn respond((status, body): (u16, String)) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, [("content-type", "application/json")], body).into_response()
}

/// The protected routes and what each requires.
///
/// # Panics
///
/// Never: the route patterns are constants, parsed in this crate's tests.
pub fn protected_routes(profile: Profile) -> Vec<ProtectedRoute> {
    let policy = |operation, purposes: &[Purpose]| RoutePolicy {
        profile,
        operation,
        purposes: purposes.to_vec(),
    };
    [
        (
            Method::GET,
            "/demo/v1/products/{id}",
            policy(Operation::Read, &[Purpose::Browse]),
        ),
        (
            Method::POST,
            "/demo/v1/checkouts",
            policy(Operation::Checkout, &[Purpose::Pay]),
        ),
        (
            Method::POST,
            "/demo/v1/orders",
            policy(Operation::Commit, &[Purpose::Pay]),
        ),
        (
            Method::GET,
            "/demo/v1/orders/{id}",
            policy(Operation::Read, &[Purpose::Browse, Purpose::Pay]),
        ),
    ]
    .into_iter()
    .map(|(m, p, policy)| ProtectedRoute::new(m, p, policy).expect("valid route pattern"))
    .collect()
}

/// The merchant's routes behind the TAP gate.
pub fn app<R, S, C>(merchant: Arc<Merchant>, gate: Arc<TapGate<R, S, C>>) -> Router
where
    R: TrustedKeyResolver + 'static,
    S: ReplayStore + 'static,
    C: Clock + 'static,
{
    Router::new()
        .route("/health", get(health))
        .route("/demo/v1/catalog", get(catalog))
        .route("/demo/v1/products/:id", get(product))
        .route("/demo/v1/checkouts", post(create_checkout))
        .route("/demo/v1/orders", post(orders::place))
        .route("/demo/v1/orders/:id", get(orders::status))
        .with_state(merchant)
        .layer(axum::middleware::from_fn_with_state(gate, guard::<R, S, C>))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

/// Anonymous browsing: allowed, and never verified-agent status.
async fn catalog(agent: Option<Extension<VerifiedAgent>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "products": CATALOG.iter().map(product_json).collect::<Vec<_>>(),
        "verifiedAgent": agent.is_some(),
    }))
}

async fn product(
    Extension(agent): Extension<VerifiedAgent>,
    Extension(id): Extension<CorrelationId>,
    Path(product_id): Path<String>,
) -> Response {
    let Some(p) = CATALOG.iter().find(|p| p.id == product_id) else {
        return respond(failure(StatusCode::NOT_FOUND, "no_such_product", &id));
    };
    Json(serde_json::json!({
        "product": product_json(p),
        "verifiedActor": {
            "keyId": agent.key_id(), "keyThumbprint": agent.key_thumbprint(),
            "provider": agent.provider(), "runtime": agent.runtime(),
            "runtimeIdentified": agent.runtime().is_some(),
            "trustAuthority": agent.authority().to_string(), "profile": agent.profile().name(),
        },
        "principalResolved": false,
        "purchaseAuthority": "none",
    }))
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckoutRequest {
    items: Vec<CheckoutItem>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckoutItem {
    id: String,
    quantity: u32,
}

async fn create_checkout(
    State(merchant): State<Arc<Merchant>>,
    Extension(agent): Extension<VerifiedAgent>,
    Extension(id): Extension<CorrelationId>,
    body: Bytes,
) -> Response {
    let scope = merchant.scope(&agent, "create-checkout");
    match merchant.store.begin(&scope, &fingerprint(&body)).await {
        Ok(Op::New | Op::InProgress { .. }) => {}
        Ok(Op::Done { status, response }) => return respond((status, response)),
        Ok(Op::Conflict) => {
            return respond(failure(StatusCode::CONFLICT, "idempotency_conflict", &id));
        }
        Err(_) => return respond(failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", &id)),
    }
    let result = checkout_for(&merchant, &agent, &body, &id).await;
    if merchant
        .store
        .finish(&scope, result.0, &result.1)
        .await
        .is_err()
    {
        return respond(failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", &id));
    }
    respond(result)
}

async fn checkout_for(
    merchant: &Merchant,
    agent: &VerifiedAgent,
    body: &[u8],
    id: &CorrelationId,
) -> (u16, String) {
    let Ok(request) = serde_json::from_slice::<CheckoutRequest>(body) else {
        return failure(StatusCode::BAD_REQUEST, "invalid_checkout", id);
    };
    if request.items.is_empty() || request.items.len() > 10 {
        return failure(StatusCode::BAD_REQUEST, "invalid_checkout", id);
    }
    let mut lines = Vec::new();
    for item in &request.items {
        let Some(p) = CATALOG.iter().find(|p| p.id == item.id) else {
            return failure(StatusCode::BAD_REQUEST, "no_such_product", id);
        };
        if !(1..=10).contains(&item.quantity) {
            return failure(StatusCode::BAD_REQUEST, "invalid_quantity", id);
        }
        lines.push(CheckoutLine {
            item_id: p.id.into(),
            quantity: item.quantity,
            unit_minor: p.unit_minor,
        });
    }
    let view = CheckoutView {
        checkout_id: uuid::Uuid::new_v4().to_string(),
        merchant: merchant.origin.as_origin(),
        total_minor: lines
            .iter()
            .map(|l| l.unit_minor * i64::from(l.quantity))
            .sum(),
        lines,
        currency: "USD".into(),
    };
    if merchant
        .store
        .put_checkout(agent.key_thumbprint(), &view)
        .await
        .is_err()
    {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "unavailable", id);
    }
    let body =
        serde_json::json!({ "checkout": view, "checkoutHash": view.hash(), "fixture": true });
    (StatusCode::CREATED.as_u16(), body.to_string())
}
