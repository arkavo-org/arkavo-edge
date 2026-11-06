//! The registry resolver against a real HTTP server: caching, revocation within the refresh
//! interval (T14), unknown-key backoff, failing closed (T13), and refusing redirects,
//! oversized documents and documents that speak for someone else (T08).

use crate::common::*;
use arkavo_tap::*;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
struct Served {
    answers: Arc<Mutex<HashMap<String, Answer>>>,
    hits: Arc<AtomicUsize>,
}

#[derive(Clone)]
enum Answer {
    Doc(Box<KeyDocument>),
    Raw(StatusCode, String),
    Redirect,
}

async fn key(State(s): State<Served>, Path(id): Path<String>) -> Response {
    s.hits.fetch_add(1, Ordering::SeqCst);
    let answer = s.answers.lock().unwrap().get(&id).cloned();
    match answer {
        Some(Answer::Doc(d)) => axum::Json(*d).into_response(),
        Some(Answer::Raw(status, body)) => (status, body).into_response(),
        Some(Answer::Redirect) => Redirect::temporary("/dev/v1/keys/agent-a").into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn serve() -> (Served, ExternalOrigin) {
    let served = Served::default();
    let app = Router::new()
        .route("/dev/v1/keys/:id", get(key))
        .with_state(served.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        served,
        ExternalOrigin::parse(&format!("http://{addr}")).unwrap(),
    )
}

fn doc(key_id: &str, seed: u8, status: &str) -> KeyDocument {
    let (signer, _) = agent(seed, key_id, KeyStatus::Active);
    KeyDocument {
        authority: "arkavo-local-demo".into(),
        environment: "local".into(),
        key_id: key_id.into(),
        kty: "OKP".into(),
        crv: "Ed25519".into(),
        x: URL_SAFE_NO_PAD.encode(signer.public_key()),
        status: status.into(),
        not_before: NOW - 10,
        not_after: NOW + 3_600,
        provider: "arkavo".into(),
        runtime: Some("runtime-1".into()),
        purposes: vec!["agent-browser-auth".into()],
        max_age_seconds: 60,
    }
}

fn config(origin: ExternalOrigin) -> RegistryConfig {
    RegistryConfig {
        origin,
        authority: authority(),
        key_path: "/dev/v1/keys/".into(),
        allow_loopback_http: true,
        timeout: Duration::from_secs(2),
        max_document_bytes: 4_096,
        max_concurrent_lookups: 4,
        unknown_key_retry_seconds: 30,
    }
}

fn served(d: KeyDocument) -> Answer {
    Answer::Doc(Box::new(d))
}

fn set(s: &Served, id: &str, a: Answer) {
    s.answers.lock().unwrap().insert(id.into(), a);
}

#[tokio::test]
async fn caches_until_refresh_and_sees_a_revocation_within_one_interval() {
    let (s, origin) = serve().await;
    set(&s, "agent-a", served(doc("agent-a", 1, "active")));
    let clock = ManualClock::new(NOW);
    let r = RegistryResolver::new(config(origin.clone()), clock.clone()).unwrap();
    let rec = r.resolve(&authority(), "agent-a").await.unwrap();
    assert_eq!(rec.status, KeyStatus::Active);
    assert_eq!(rec.binding.runtime.as_deref(), Some("runtime-1"));
    assert_eq!(rec.refresh_by, NOW + 60);
    assert!(rec.provenance.ends_with("/dev/v1/keys/agent-a"));
    r.resolve(&authority(), "agent-a").await.unwrap();
    assert_eq!(s.hits.load(Ordering::SeqCst), 1, "served from cache");

    set(&s, "agent-a", served(doc("agent-a", 1, "revoked")));
    clock.advance(59);
    assert_eq!(
        r.resolve(&authority(), "agent-a").await.unwrap().status,
        KeyStatus::Active
    );
    clock.advance(1);
    assert_eq!(
        r.resolve(&authority(), "agent-a").await.unwrap().status,
        KeyStatus::Revoked
    );
    assert_eq!(s.hits.load(Ordering::SeqCst), 2);

    // Once stale, a failed lookup fails closed instead of reviving the cached record.
    set(
        &s,
        "agent-a",
        Answer::Raw(StatusCode::INTERNAL_SERVER_ERROR, "down".into()),
    );
    clock.advance(60);
    assert!(matches!(
        r.resolve(&authority(), "agent-a").await,
        Err(ResolveError::Unavailable(_))
    ));
}

#[tokio::test]
async fn unknown_keys_back_off_and_other_authorities_are_never_asked() {
    let (s, origin) = serve().await;
    let clock = ManualClock::new(NOW);
    let r = RegistryResolver::new(config(origin), clock.clone()).unwrap();
    assert_eq!(
        r.resolve(&authority(), "nobody").await.unwrap_err(),
        ResolveError::Unknown
    );
    assert_eq!(
        r.resolve(&authority(), "nobody").await.unwrap_err(),
        ResolveError::Unknown
    );
    assert_eq!(s.hits.load(Ordering::SeqCst), 1);
    clock.advance(30);
    r.resolve(&authority(), "nobody").await.unwrap_err();
    assert_eq!(s.hits.load(Ordering::SeqCst), 2);

    let other = TrustAuthority {
        name: "someone-else".into(),
        environment: Environment::Local,
    };
    assert_eq!(
        r.resolve(&other, "agent-a").await.unwrap_err(),
        ResolveError::Unknown
    );
    assert_eq!(
        r.resolve(&authority(), "../etc").await.unwrap_err(),
        ResolveError::Unknown
    );
    assert_eq!(s.hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unusable_answers_fail_closed() {
    let (s, origin) = serve().await;
    let r = RegistryResolver::new(config(origin), ManualClock::new(NOW)).unwrap();
    let mut wrong_authority = doc("k1", 1, "active");
    wrong_authority.authority = "someone-else".into();
    let mut wrong_env = doc("k2", 1, "active");
    wrong_env.environment = "production".into();
    let wrong_key = doc("other", 1, "active");
    let mut wrong_type = doc("k4", 1, "active");
    wrong_type.kty = "EC".into();
    let mut wrong_age = doc("k5", 1, "active");
    wrong_age.max_age_seconds = 86_400;
    let mut wrong_status = doc("k6", 1, "active");
    wrong_status.status = "paused".into();
    let mut wrong_purpose = doc("k7", 1, "active");
    wrong_purpose.purposes = vec!["anything".into()];
    let mut wrong_x = doc("k8", 1, "active");
    wrong_x.x = "!!".into();
    let answers = [
        ("k1", served(wrong_authority)),
        ("k2", served(wrong_env)),
        ("k3", served(wrong_key)),
        ("k4", served(wrong_type)),
        ("k5", served(wrong_age)),
        ("k6", served(wrong_status)),
        ("k7", served(wrong_purpose)),
        ("k8", served(wrong_x)),
        ("k9", Answer::Raw(StatusCode::OK, "{not json".into())),
        ("k10", Answer::Raw(StatusCode::OK, "x".repeat(5_000))),
        ("k11", Answer::Redirect),
        (
            "k12",
            Answer::Raw(StatusCode::SERVICE_UNAVAILABLE, String::new()),
        ),
    ];
    for (id, answer) in answers {
        set(&s, id, answer);
        assert!(
            matches!(
                r.resolve(&authority(), id).await,
                Err(ResolveError::Unavailable(_))
            ),
            "{id}"
        );
    }
}

#[tokio::test]
async fn an_unreachable_registry_is_unavailable() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let origin = ExternalOrigin::parse(&format!("http://{addr}")).unwrap();
    let r = RegistryResolver::new(config(origin), ManualClock::new(NOW)).unwrap();
    assert!(matches!(
        r.resolve(&authority(), "agent-a").await,
        Err(ResolveError::Unavailable(_))
    ));
}

#[test]
fn plain_http_is_only_for_loopback_development() {
    let clock = ManualClock::new(NOW);
    let mut c = config(ExternalOrigin::parse("http://registry.example").unwrap());
    assert!(RegistryResolver::new(c.clone(), clock.clone()).is_err());
    c.origin = ExternalOrigin::parse("http://127.0.0.1:9002").unwrap();
    c.allow_loopback_http = false;
    assert!(RegistryResolver::new(c.clone(), clock.clone()).is_err());
    c.origin = ExternalOrigin::parse("https://registry.example").unwrap();
    assert!(RegistryResolver::new(c.clone(), clock.clone()).is_ok());
    c.key_path = "keys".into();
    assert!(RegistryResolver::new(c, clock).is_err());
}
