//! In-process fake Creator + IdP driving `IdentitySession::bearer`.
//!
//! Interactive login uses production `loopback::bind` on ports 52171–52178, so
//! these tests take a process-wide mutex. The mock launcher GETs the loopback
//! callback and must return before that GET completes: `IdentitySession`
//! awaits `launch` and only then `wait_for_callback`.

#![allow(clippy::disallowed_methods)] // #[tokio::test] expands to Runtime::block_on

use arkavo_identity::{
    Clock, IdentityError, IdentitySession, LOOPBACK_PORTS, Launcher, Prompt, SessionConfig, load,
};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

static LOOPBACK: Mutex<()> = Mutex::const_new(());

const AUTH_CODE: &str = "fixed";
const ACCESS_TOKEN: &str = "test-access-token";
const REFRESH_TOKEN: &str = "test-refresh-token";

struct WallClock;

impl Clock for WallClock {
    fn now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

enum LaunchKind {
    Code,
    Denied,
}

struct FakeCreator {
    kind: LaunchKind,
    launches: Arc<AtomicUsize>,
}

impl Launcher for FakeCreator {
    fn launch<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), IdentityError>> + Send + 'a>> {
        Box::pin(async move {
            self.launches.fetch_add(1, Ordering::SeqCst);
            let (redirect_uri, state) = parse_creator_query(url)?;
            let query = match self.kind {
                LaunchKind::Code => format!("code={AUTH_CODE}&state={state}"),
                LaunchKind::Denied => format!("error=access_denied&state={state}"),
            };
            let callback = format!("{redirect_uri}?{query}");
            tokio::spawn(async move {
                let _ = callback_get(&callback).await;
            });
            Ok(())
        })
    }
}

struct FakeIdp {
    addr: std::net::SocketAddr,
    authorize_hits: Arc<AtomicUsize>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for FakeIdp {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

struct Fixture {
    session: Arc<IdentitySession>,
    token_path: std::path::PathBuf,
    launches: Arc<AtomicUsize>,
    authorize_hits: Arc<AtomicUsize>,
    _tmp: tempfile::TempDir,
    _idp: FakeIdp,
}

#[tokio::test]
async fn bearer_interactive_completes_one_ceremony() {
    let _ports = LOOPBACK.lock().await;
    tokio::time::timeout(Duration::from_secs(15), async {
        let fx = fixture(LaunchKind::Code).await;
        let token = fx
            .session
            .bearer(Prompt::Interactive)
            .await
            .expect("interactive ceremony");
        assert!(!token.is_empty(), "access token must be non-empty");
        let stored = load(&fx.token_path)
            .expect("load store")
            .expect("store written");
        assert_eq!(stored.refresh_token.as_deref(), Some(REFRESH_TOKEN));
        assert_eq!(fx.launches.load(Ordering::SeqCst), 1);
        assert_eq!(
            fx.authorize_hits.load(Ordering::SeqCst),
            0,
            "Creator stand-in must not GET /oauth/authorize"
        );
    })
    .await
    .expect("timed out");
}

#[tokio::test]
async fn concurrent_bearers_share_one_ceremony() {
    let _ports = LOOPBACK.lock().await;
    tokio::time::timeout(Duration::from_secs(15), async {
        let fx = fixture(LaunchKind::Code).await;
        let a = Arc::clone(&fx.session);
        let b = Arc::clone(&fx.session);
        let (left, right) = tokio::join!(
            async move { a.bearer(Prompt::Interactive).await },
            async move { b.bearer(Prompt::Interactive).await },
        );
        let left = left.expect("first concurrent bearer");
        let right = right.expect("second concurrent bearer");
        assert!(!left.is_empty() && !right.is_empty());
        assert_eq!(fx.launches.load(Ordering::SeqCst), 1);
        assert_eq!(fx.authorize_hits.load(Ordering::SeqCst), 0);
    })
    .await
    .expect("timed out");
}

#[tokio::test]
async fn user_denial_is_access_denied() {
    let _ports = LOOPBACK.lock().await;
    tokio::time::timeout(Duration::from_secs(15), async {
        let fx = fixture(LaunchKind::Denied).await;
        let err = fx
            .session
            .bearer(Prompt::Interactive)
            .await
            .expect_err("denial must fail");
        assert!(
            matches!(err, IdentityError::AccessDenied),
            "expected AccessDenied"
        );
        assert_eq!(fx.launches.load(Ordering::SeqCst), 1);
    })
    .await
    .expect("timed out");
}

async fn fixture(kind: LaunchKind) -> Fixture {
    let tmp = tempfile::tempdir().expect("tempdir");
    let token_path = tmp.path().join("identity_token");
    let idp = spawn_idp().await;
    let launches = Arc::new(AtomicUsize::new(0));
    let launcher: Arc<dyn Launcher> = Arc::new(FakeCreator {
        kind,
        launches: Arc::clone(&launches),
    });
    let session = IdentitySession::with_config(SessionConfig {
        http: http_client(),
        platform_url: format!("http://{}", idp.addr),
        identity_host: "127.0.0.1".into(),
        token_path: token_path.clone(),
        clock: Arc::new(WallClock),
        launcher,
    });
    Fixture {
        session: Arc::new(session),
        token_path,
        launches,
        authorize_hits: Arc::clone(&idp.authorize_hits),
        _tmp: tmp,
        _idp: idp,
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client")
}

async fn spawn_idp() -> FakeIdp {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fake IdP");
    let addr = listener.local_addr().expect("local_addr");
    let authorize_hits = Arc::new(AtomicUsize::new(0));
    let hits = Arc::clone(&authorize_hits);
    let well_known = format!(
        r#"{{"idp":{{"issuer":"http://{addr}","authorization_endpoint":"http://{addr}/oauth/authorize","token_endpoint":"http://{addr}/oauth/token"}},"kas":{{"uri":"https://platform.arkavo.net"}}}}"#
    );
    let handle = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let well_known = well_known.clone();
            let hits = Arc::clone(&hits);
            tokio::spawn(async move {
                serve_idp(stream, &well_known, &hits).await;
            });
        }
    });
    FakeIdp {
        addr,
        authorize_hits,
        handle,
    }
}

async fn serve_idp(mut stream: TcpStream, well_known: &str, authorize_hits: &AtomicUsize) {
    let req = read_request(&mut stream).await;
    let first = req.lines().next().unwrap_or("");
    if first.starts_with("GET /.well-known/opentdf-configuration") {
        write_http(&mut stream, 200, "application/json", well_known).await;
        return;
    }
    if first.starts_with("GET /oauth/authorize") || first.starts_with("POST /oauth/authorize") {
        authorize_hits.fetch_add(1, Ordering::SeqCst);
        write_http(&mut stream, 404, "application/json", "{}").await;
        return;
    }
    if first.starts_with("POST /oauth/token") {
        let body = http_body(&req);
        if let Some(json) = token_json(body) {
            write_http(&mut stream, 200, "application/json", &json).await;
        } else {
            write_http(
                &mut stream,
                400,
                "application/json",
                r#"{"error":"invalid_request"}"#,
            )
            .await;
        }
        return;
    }
    write_http(&mut stream, 404, "application/json", "{}").await;
}

fn token_json(body: &str) -> Option<String> {
    let form = parse_form(body);
    let grant = form.get("grant_type").map(String::as_str)?;
    let client_id = form.get("client_id").map(String::as_str)?;
    let redirect_uri = form.get("redirect_uri").map(String::as_str)?;
    let verifier = form.get("code_verifier").map(String::as_str)?;
    if grant != "authorization_code"
        || client_id != "arkavo-edge"
        || verifier.is_empty()
        || !is_registered_loopback(redirect_uri)
    {
        return None;
    }
    Some(format!(
        r#"{{"access_token":"{ACCESS_TOKEN}","refresh_token":"{REFRESH_TOKEN}","expires_in":3600}}"#
    ))
}

fn is_registered_loopback(uri: &str) -> bool {
    LOOPBACK_PORTS
        .into_iter()
        .any(|port| uri == format!("http://127.0.0.1:{port}/cb"))
}

fn parse_form(body: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for pair in body.split('&') {
        if pair.is_empty() {
            continue;
        }
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let Some(k) = form_decode(k) else { continue };
        let Some(v) = form_decode(v) else { continue };
        out.insert(k, v);
    }
    out
}

fn form_decode(input: &str) -> Option<String> {
    percent_decode(&input.replace('+', " "))
}

fn parse_creator_query(url: &str) -> Result<(String, String), IdentityError> {
    if !url.starts_with("arkavocreator://oauth/authorize?") {
        return Err(IdentityError::Transport(
            "launcher url must be arkavocreator://oauth/authorize".into(),
        ));
    }
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut redirect_uri = None;
    let mut state = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let k =
            percent_decode(k).ok_or_else(|| IdentityError::Transport("bad query key".into()))?;
        let v =
            percent_decode(v).ok_or_else(|| IdentityError::Transport("bad query value".into()))?;
        match k.as_str() {
            "redirect_uri" => redirect_uri = Some(v),
            "state" => state = Some(v),
            _ => {}
        }
    }
    match (redirect_uri, state) {
        (Some(redirect_uri), Some(state)) => Ok((redirect_uri, state)),
        _ => Err(IdentityError::Transport(
            "creator url missing redirect_uri or state".into(),
        )),
    }
}

async fn callback_get(target: &str) -> std::io::Result<()> {
    let rest = target.strip_prefix("http://").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "callback must be http")
    })?;
    let (hostport, path) = rest.split_once('/').ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "callback missing path")
    })?;
    let (host, port) = hostport.split_once(':').ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "callback missing port")
    })?;
    let port: u16 = port
        .parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut stream = TcpStream::connect((host, port)).await?;
    let req = format!("GET /{path} HTTP/1.1\r\nHost: {hostport}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await?;
    let mut buf = [0u8; 256];
    let _ = stream.read(&mut buf).await;
    Ok(())
}

async fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = stream.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(header_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&buf[..header_end]).unwrap_or("");
            let content_length = headers.lines().find_map(|line| {
                let (k, v) = line.split_once(':')?;
                k.trim()
                    .eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse::<usize>().ok())
                    .flatten()
            });
            let body_start = header_end + 4;
            let want = content_length.unwrap_or(0);
            while buf.len() < body_start + want {
                let n = stream.read(&mut tmp).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
            }
            break;
        }
        if buf.len() > 64 * 1024 {
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn http_body(req: &str) -> &str {
    req.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("")
}

async fn write_http(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(header.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hi = from_hex(bytes[i + 1])?;
                let lo = from_hex(bytes[i + 2])?;
                out.push((hi << 4) | lo);
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
