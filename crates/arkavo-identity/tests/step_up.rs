//! `IdentitySession::step_up` against an in-process Creator and identity.
//!
//! A ceremony binds the production loopback ports (52171–52178), and the
//! library's own tests assert on which of them are free, so these run as
//! their own test binary and one at a time.

#![allow(clippy::disallowed_methods)] // #[tokio::test] expands to Runtime::block_on

use arkavo_identity::{
    Clock, DELEGATE_SCOPE, IdentityError, IdentitySession, Launcher, Prompt, SessionConfig,
};
use arkavo_test_macros::spec;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

static LOOPBACK: Mutex<()> = Mutex::const_new(());

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now(&self) -> i64 {
        self.0
    }
}

/// Plays Arkavo Creator: records the scope the CLI asked for and answers the
/// loopback with `answer` (`code=…` or `error=…`) and the state.
struct CreatorStandIn {
    seen_scope: Arc<std::sync::Mutex<String>>,
    answer: &'static str,
}

impl Launcher for CreatorStandIn {
    fn launch<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), IdentityError>> + Send + 'a>> {
        let query: HashMap<String, String> = url::Url::parse(url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        self.seen_scope.lock().unwrap().clone_from(&query["scope"]);
        let callback = format!(
            "{}?{}&state={}",
            query["redirect_uri"], self.answer, query["state"]
        );
        // The session awaits `launch` before it listens for the callback, so
        // the GET must not be awaited here.
        Box::pin(async move {
            tokio::spawn(async move {
                let _ = reqwest::get(callback).await;
            });
            Ok(())
        })
    }
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if buf.len() >= end + 4 + length {
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

async fn write_json(stream: &mut TcpStream, body: &str) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// An identity server whose token endpoint grants `scope` with `access`.
async fn serve_idp_granting(access: &str, scope: &str) -> std::net::SocketAddr {
    let token = serde_json::json!({
        "access_token": access,
        "refresh_token": "delegate-refresh",
        "expires_in": 3600,
        "scope": scope,
    })
    .to_string();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let well_known = serde_json::json!({
        "idp": {
            "issuer": format!("http://{addr}"),
            "authorization_endpoint": format!("http://{addr}/oauth/authorize"),
            "token_endpoint": format!("http://{addr}/oauth/token"),
        },
        "kas": { "uri": "https://platform.arkavo.net" }
    })
    .to_string();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let request = read_request(&mut stream).await;
            if request.starts_with("POST ") {
                write_json(&mut stream, &token).await;
            } else {
                write_json(&mut stream, &well_known).await;
            }
        }
    });
    addr
}

fn session(
    addr: std::net::SocketAddr,
    token_path: std::path::PathBuf,
    seen_scope: Arc<std::sync::Mutex<String>>,
    answer: &'static str,
) -> IdentitySession {
    IdentitySession::with_config(SessionConfig {
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap(),
        platform_url: format!("http://{addr}"),
        identity_host: "127.0.0.1".into(),
        token_path,
        clock: Arc::new(FixedClock(0)),
        launcher: Arc::new(CreatorStandIn { seen_scope, answer }),
    })
}

/// A token that can authorize agents must not outlive the command that asked
/// for it: it is never written over the everyday session, and the request
/// carries no `offline_access`, so no refresh token can renew it.
#[spec("IDS-022")]
#[tokio::test]
async fn step_up_asks_for_the_scope_alone_and_stores_nothing() {
    let _ports = LOOPBACK.lock().await;
    let addr = serve_idp_granting("delegate-access", "openid agents:delegate").await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("identity_token");
    let everyday = br#"{"access_token":"everyday","refresh_token":"r","expires_at":9000}"#;
    std::fs::write(&path, everyday).unwrap();
    let seen = Arc::new(std::sync::Mutex::new(String::new()));
    let session = session(addr, path.clone(), seen.clone(), "code=c");

    let token = session.step_up(DELEGATE_SCOPE).await.unwrap();

    assert_eq!(token, "delegate-access");
    assert_eq!(*seen.lock().unwrap(), "openid agents:delegate");
    assert_eq!(std::fs::read(&path).unwrap(), everyday);
    // Later commands are still handed the everyday session.
    assert_eq!(session.bearer(Prompt::Never).await.unwrap(), "everyday");
}

/// Identity names what it granted. A token without the scope would be
/// refused at the endpoint it was fetched for, with a worse message.
#[spec("IDS-022")]
#[tokio::test]
async fn step_up_refuses_a_token_that_was_not_granted_the_scope() {
    let _ports = LOOPBACK.lock().await;
    let addr = serve_idp_granting("plain-access", "openid").await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(addr, dir.path().join("t"), Arc::default(), "code=c");

    match session.step_up(DELEGATE_SCOPE).await {
        Err(IdentityError::Token(msg)) => {
            assert!(msg.contains("did not grant agents:delegate"), "{msg}");
        }
        other => panic!("expected a scope refusal, got {other:?}"),
    }
}

/// Identity answers a scope it will not give this client on the redirect.
/// That is not the person declining, and must not be reported as one.
#[spec("IDS-013")]
#[tokio::test]
async fn a_refusal_from_identity_is_not_reported_as_the_person_declining() {
    let _ports = LOOPBACK.lock().await;
    let addr = serve_idp_granting("unused", "openid").await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(
        addr,
        dir.path().join("t"),
        Arc::default(),
        "error=invalid_scope",
    );

    match session.step_up(DELEGATE_SCOPE).await {
        Err(IdentityError::Token(msg)) => assert!(msg.contains("invalid_scope"), "{msg}"),
        other => panic!("expected identity's refusal, got {other:?}"),
    }
}

#[spec("IDS-013")]
#[tokio::test]
async fn the_person_declining_is_still_access_denied() {
    let _ports = LOOPBACK.lock().await;
    let addr = serve_idp_granting("unused", "openid").await;
    let dir = tempfile::tempdir().unwrap();
    let session = session(
        addr,
        dir.path().join("t"),
        Arc::default(),
        "error=access_denied",
    );

    assert!(matches!(
        session.step_up(DELEGATE_SCOPE).await,
        Err(IdentityError::AccessDenied)
    ));
}
