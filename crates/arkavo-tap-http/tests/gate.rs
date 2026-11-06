//! The gate in front of a real axum router: verified context only for verified requests
//! on protected routes (T01, T02, T15, T30), and the transport limits before them.
#![allow(clippy::disallowed_methods)] // Tokio test entrypoints own their runtime.

use arkavo_tap::*;
use arkavo_tap_http::{
    CORRELATION_HEADER, CorrelationId, GateConfig, ProtectedRoute, TapGate, guard,
};
use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Extension;
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::routing::{get, post};
use std::future::Future;
use std::sync::Arc;
use tower::ServiceExt as _;

const NOW: i64 = 1_893_456_000;
const ORIGIN: &str = "http://127.0.0.1:3001";

fn authority() -> TrustAuthority {
    TrustAuthority {
        name: "arkavo-local-demo".into(),
        environment: Environment::Local,
    }
}

struct OneKey(KeyRecord);

impl TrustedKeyResolver for OneKey {
    fn resolve(
        &self,
        authority: &TrustAuthority,
        key_id: &str,
    ) -> impl Future<Output = Result<KeyRecord, ResolveError>> + Send {
        let found = *authority == self.0.authority && key_id == self.0.key_id;
        std::future::ready(if found {
            Ok(self.0.clone())
        } else {
            Err(ResolveError::Unknown)
        })
    }
}

fn signer() -> Ed25519Signer {
    Ed25519Signer::new(
        &[5u8; 32],
        "agent-a",
        SigningPolicy {
            destinations: vec![ORIGIN.into()],
            operations: vec![Operation::Read, Operation::Checkout],
        },
    )
}

async fn app(dir: &tempfile::TempDir, max_body: usize, rate: u32) -> Router {
    let s = signer();
    let record = KeyRecord::ed25519(KeyFields {
        authority: authority(),
        key_id: "agent-a".into(),
        binding: ActorBinding {
            provider: "arkavo".into(),
            runtime: None,
        },
        public_key: &s.public_key(),
        status: KeyStatus::Active,
        not_before: 0,
        not_after: i64::MAX,
        purposes: vec![Purpose::Browse, Purpose::Pay],
        retrieved_at: NOW,
        refresh_by: i64::MAX,
        provenance: "test".into(),
    })
    .unwrap();
    let clock = ManualClock::new(NOW);
    let replay = SqliteReplayStore::open(&dir.path().join("r.sqlite"), clock.clone(), 30)
        .await
        .unwrap();
    let origin = ExternalOrigin::parse(ORIGIN).unwrap();
    let verifier = TapVerifier::new(
        VerifierConfig {
            authority: authority(),
            audience: origin.clone(),
            max_header_bytes: 16 * 1024,
        },
        OneKey(record),
        replay,
        clock,
    );
    let routes = vec![
        ProtectedRoute::new(
            Method::GET,
            "/demo/v1/products/{id}",
            RoutePolicy {
                profile: Profile::ArkavoRfc9421V1,
                operation: Operation::Read,
                purposes: vec![Purpose::Browse],
            },
        )
        .unwrap(),
        ProtectedRoute::new(
            Method::POST,
            "/demo/v1/checkouts",
            RoutePolicy {
                profile: Profile::ArkavoRfc9421V1,
                operation: Operation::Checkout,
                purposes: vec![Purpose::Pay],
            },
        )
        .unwrap(),
    ];
    let gate = Arc::new(TapGate::new(
        verifier,
        GateConfig {
            origin,
            max_body_bytes: max_body,
            max_verifications_per_second: rate,
        },
        routes,
    ));
    Router::new()
        .route("/demo/v1/products/:id", get(product))
        .route("/demo/v1/checkouts", post(checkout))
        .route("/public", get(public))
        .layer(axum::middleware::from_fn_with_state(gate, guard))
}

async fn product(
    agent: Option<Extension<VerifiedAgent>>,
    Extension(id): Extension<CorrelationId>,
) -> String {
    let agent = agent.expect("protected handlers only run for verified agents");
    assert_eq!(agent.decision_id(), id.0);
    agent.key_id().to_owned()
}

async fn checkout(Extension(agent): Extension<VerifiedAgent>, body: Bytes) -> String {
    format!("{} {}", agent.purpose().tag(), body.len())
}

async fn public(agent: Option<Extension<VerifiedAgent>>, headers: HeaderMap) -> String {
    format!(
        "verified={} spoofed={}",
        agent.is_some(),
        headers.contains_key("x-agent-verified")
    )
}

fn signed(method: &str, path: &str, body: &[u8], op: Operation, purpose: Purpose) -> Request<Body> {
    let mut headers = Vec::new();
    if op != Operation::Read {
        headers.push(("content-type".to_owned(), "application/json".to_owned()));
        headers.push((
            "content-digest".to_owned(),
            arkavo_tap::digest::content_digest(body),
        ));
        headers.push(("idempotency-key".to_owned(), "idem-1".to_owned()));
    }
    let ctx = RequestContext::new(
        ExternalOrigin::parse(ORIGIN).unwrap(),
        method,
        path,
        headers.clone(),
    )
    .unwrap();
    let prepared = arkavo_tap::signer::prepare(
        ctx,
        Profile::ArkavoRfc9421V1,
        op,
        purpose,
        "agent-a",
        NOW,
        120,
    )
    .unwrap();
    let sig = arkavo_tap::signer::sign(&signer(), &prepared).unwrap();
    let mut builder = Request::builder().method(method).uri(path);
    for (n, v) in headers {
        builder = builder.header(n, v);
    }
    builder
        .header("signature-input", sig.signature_input)
        .header("signature", sig.signature)
        .body(Body::from(body.to_vec()))
        .unwrap()
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, String, bool) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let has_id = response.headers().contains_key(CORRELATION_HEADER);
    let body = to_bytes(response.into_body(), 1 << 16).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap(), has_id)
}

#[tokio::test]
async fn t01_a_verified_read_reaches_the_handler_with_its_agent() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, 1024, 100).await;
    let req = signed(
        "GET",
        "/demo/v1/products/demo-001",
        b"",
        Operation::Read,
        Purpose::Browse,
    );
    let (status, body, has_id) = send(&app, req).await;
    assert_eq!(
        (status, body.as_str(), has_id),
        (StatusCode::OK, "agent-a", true)
    );
}

#[tokio::test]
async fn t02_t15_unsigned_or_spoofed_requests_never_reach_protected_handlers() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, 1024, 100).await;
    let forged = Request::get("/demo/v1/products/demo-001")
        .header("x-agent-verified", "true")
        .header("x-tap-key-id", "agent-a")
        .body(Body::empty())
        .unwrap();
    let (status, body, has_id) = send(&app, forged).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.contains("\"error\":\"missing_proof\""), "{body}");
    assert!(body.contains("correlationId") && has_id);
}

#[tokio::test]
async fn other_methods_on_a_protected_path_are_protected_too() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, 1024, 100).await;
    for method in [Method::HEAD, Method::DELETE] {
        let req = Request::builder()
            .method(method.clone())
            .uri("/demo/v1/products/demo-001")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            send(&app, req).await.0,
            StatusCode::UNAUTHORIZED,
            "{method}"
        );
    }
}

#[tokio::test]
async fn t30_anonymous_routes_browse_without_verified_status() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, 1024, 100).await;
    let mut req = signed("GET", "/public", b"", Operation::Read, Purpose::Browse);
    req.headers_mut()
        .insert("x-agent-verified", "true".parse().unwrap());
    let (status, body, _) = send(&app, req).await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, "verified=false spoofed=false")
    );
}

#[tokio::test]
async fn writes_are_verified_over_the_exact_content_the_handler_sees() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, 1024, 100).await;
    let body = br#"{"items":[{"id":"demo-001","quantity":1}]}"#;
    let (status, text, _) = send(
        &app,
        signed(
            "POST",
            "/demo/v1/checkouts",
            body,
            Operation::Checkout,
            Purpose::Pay,
        ),
    )
    .await;
    assert_eq!(
        (status, text),
        (StatusCode::OK, format!("agent-payer-auth {}", body.len()))
    );
    let mut altered = signed(
        "POST",
        "/demo/v1/checkouts",
        body,
        Operation::Checkout,
        Purpose::Pay,
    );
    *altered.body_mut() = Body::from(&br#"{"items":[{"id":"demo-001","quantity":9}]}"#[..]);
    assert_eq!(send(&app, altered).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn content_and_rate_limits_come_first() {
    let dir = tempfile::tempdir().unwrap();
    let small = app(&dir, 8, 100).await;
    let body = br#"{"items":[]}"#;
    let declared = signed(
        "POST",
        "/demo/v1/checkouts",
        body,
        Operation::Checkout,
        Purpose::Pay,
    );
    let mut with_length = declared;
    with_length
        .headers_mut()
        .insert("content-length", body.len().to_string().parse().unwrap());
    assert_eq!(
        send(&small, with_length).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let streamed = signed(
        "POST",
        "/demo/v1/checkouts",
        body,
        Operation::Checkout,
        Purpose::Pay,
    );
    assert_eq!(
        send(&small, streamed).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );

    // Three requests span at most two one-second windows, so one is over a limit of one.
    let dir = tempfile::tempdir().unwrap();
    let limited = app(&dir, 1024, 1).await;
    let mut statuses = Vec::new();
    for path in [
        "/demo/v1/products/a",
        "/demo/v1/products/b",
        "/demo/v1/products/c",
    ] {
        let req = signed("GET", path, b"", Operation::Read, Purpose::Browse);
        statuses.push(send(&limited, req).await.0);
    }
    assert!(
        statuses.contains(&StatusCode::TOO_MANY_REQUESTS),
        "{statuses:?}"
    );
}

#[tokio::test]
async fn non_ascii_header_values_cannot_be_covered_and_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let app = app(&dir, 1024, 100).await;
    let mut req = signed(
        "GET",
        "/demo/v1/products/demo-001",
        b"",
        Operation::Read,
        Purpose::Browse,
    );
    req.headers_mut().insert(
        "x-note",
        axum::http::HeaderValue::from_bytes(b"caf\xc3\xa9").unwrap(),
    );
    assert_eq!(send(&app, req).await.0, StatusCode::OK);
}
