//! OIDC session: cache, refresh-before-expiry, Creator ceremony.

use crate::discovery::{DEFAULT_IDENTITY_HOST, DEFAULT_PLATFORM_URL};
use crate::error::{IdentityError, Prompt};
use crate::store::StoredTokens;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub trait Clock: Send + Sync {
    fn now(&self) -> i64;
}

pub trait Launcher: Send + Sync {
    fn launch<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), IdentityError>> + Send + 'a>>;
}

pub struct SessionConfig {
    pub http: reqwest::Client,
    pub platform_url: String,
    pub identity_host: String,
    pub token_path: PathBuf,
    pub clock: Arc<dyn Clock>,
    pub launcher: Arc<dyn Launcher>,
}

struct CachedAccess {
    access_token: String,
    expires_at: i64,
}

pub struct IdentitySession {
    http: reqwest::Client,
    platform_url: String,
    identity_host: String,
    token_path: Option<PathBuf>,
    clock: Arc<dyn Clock>,
    launcher: Arc<dyn Launcher>,
    ceremony: tokio::sync::Mutex<()>,
    cache: Mutex<Option<CachedAccess>>,
    force_refresh: AtomicBool,
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

struct CreatorLauncher;

impl Launcher for CreatorLauncher {
    fn launch<'a>(
        &'a self,
        url: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), IdentityError>> + Send + 'a>> {
        Box::pin(crate::broker::launch_creator(url))
    }
}

impl Default for IdentitySession {
    fn default() -> Self {
        Self::new()
    }
}

impl IdentitySession {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            http,
            platform_url: env_or("ARKAVO_PLATFORM_URL", DEFAULT_PLATFORM_URL),
            identity_host: env_or("ARKAVO_IDENTITY_HOST", DEFAULT_IDENTITY_HOST),
            token_path: crate::store::token_path().ok(),
            clock: Arc::new(SystemClock),
            launcher: Arc::new(CreatorLauncher),
            ceremony: tokio::sync::Mutex::new(()),
            cache: Mutex::new(None),
            force_refresh: AtomicBool::new(false),
        }
    }

    pub fn with_config(cfg: SessionConfig) -> Self {
        Self {
            http: cfg.http,
            platform_url: cfg.platform_url,
            identity_host: cfg.identity_host,
            token_path: Some(cfg.token_path),
            clock: cfg.clock,
            launcher: cfg.launcher,
            ceremony: tokio::sync::Mutex::new(()),
            cache: Mutex::new(None),
            force_refresh: AtomicBool::new(false),
        }
    }

    fn token_file(&self) -> Result<PathBuf, IdentityError> {
        self.token_path
            .clone()
            .ok_or_else(|| IdentityError::Store("Could not determine data directory".into()))
    }

    pub async fn bearer(&self, prompt: Prompt) -> Result<String, IdentityError> {
        let path = self.token_file()?;
        let _guard = self.ceremony.lock().await;
        let force = self.force_refresh.load(Ordering::SeqCst);
        if !force && let Some(token) = self.cached_access() {
            return Ok(token);
        }
        let stored = match crate::store::load(&path) {
            Ok(tokens) => tokens,
            Err(IdentityError::Store(msg)) if msg.contains("parse token file") => {
                let _ = crate::store::delete(&path);
                self.clear_cache();
                None
            }
            Err(e) => return Err(e),
        };
        if !force
            && let Some(tokens) = stored.as_ref()
            && !tokens.access_token.is_empty()
            && self.is_fresh(tokens.expires_at)
        {
            self.remember(tokens);
            return Ok(tokens.access_token.clone());
        }
        if let Some(refresh_token) = stored
            .as_ref()
            .and_then(|t| t.refresh_token.clone())
            .filter(|s| !s.is_empty())
        {
            match self.refresh_and_persist(&refresh_token).await {
                Ok(access) => {
                    self.force_refresh.store(false, Ordering::SeqCst);
                    return Ok(access);
                }
                Err(IdentityError::Token(_) | IdentityError::Transport(_)) => {
                    let _ = crate::store::delete(&path);
                    self.clear_cache();
                }
                Err(e) => return Err(e),
            }
        }
        match prompt {
            Prompt::Never => Err(IdentityError::LoginRequired("run 'arkavo login'".into())),
            Prompt::Interactive => {
                let access = self.interactive_login().await?;
                self.force_refresh.store(false, Ordering::SeqCst);
                Ok(access)
            }
        }
    }

    pub async fn login(&self) -> Result<String, IdentityError> {
        let access = self.bearer(Prompt::Interactive).await?;
        crate::cwt::sub(&access)
    }

    pub async fn logout(&self) -> Result<(), IdentityError> {
        let path = self.token_file()?;
        let _guard = self.ceremony.lock().await;
        self.force_refresh.store(false, Ordering::SeqCst);
        self.clear_cache();
        crate::store::delete(&path)
    }

    pub fn invalidate(&self) {
        // The token file still holds the rejected access; the next bearer
        // must refresh (or login) instead of reloading it.
        self.force_refresh.store(true, Ordering::SeqCst);
        self.clear_cache();
    }

    fn is_fresh(&self, expires_at: i64) -> bool {
        self.clock.now().saturating_add(60) < expires_at
    }

    fn cached_access(&self) -> Option<String> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let token = cache.as_ref().and_then(|cached| {
            self.is_fresh(cached.expires_at)
                .then(|| cached.access_token.clone())
        });
        drop(cache);
        token
    }

    fn remember(&self, tokens: &StoredTokens) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some(CachedAccess {
            access_token: tokens.access_token.clone(),
            expires_at: tokens.expires_at,
        });
    }

    fn clear_cache(&self) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    async fn refresh_and_persist(&self, refresh_token: &str) -> Result<String, IdentityError> {
        let endpoints =
            crate::discovery::discover(&self.http, &self.platform_url, &self.identity_host).await?;
        let mut tokens =
            crate::token::refresh(&self.http, &endpoints.token_endpoint, refresh_token).await?;
        if tokens.refresh_token.is_none() {
            tokens.refresh_token = Some(refresh_token.to_owned());
        }
        crate::store::save(&tokens, &self.token_file()?)?;
        self.remember(&tokens);
        Ok(tokens.access_token)
    }

    async fn interactive_login(&self) -> Result<String, IdentityError> {
        let endpoints =
            crate::discovery::discover(&self.http, &self.platform_url, &self.identity_host).await?;
        let bound = crate::loopback::bind()?;
        let redirect_uri = bound.redirect_uri.clone();
        let pkce = crate::pkce::Pkce::generate();
        println!(
            "Pairing code: {}",
            crate::pkce::Pkce::pairing_code(&pkce.state)
        );
        let authorize = crate::broker::authorize_url(&endpoints, &pkce, &redirect_uri);
        let creator = crate::broker::creator_url(&authorize);
        self.launcher.launch(&creator).await?;
        let callback = crate::loopback::wait_for_callback(
            bound,
            &pkce.state,
            crate::loopback::CALLBACK_DEADLINE,
        )
        .await?;
        match callback {
            crate::loopback::Callback::Error { .. } => Err(IdentityError::AccessDenied),
            crate::loopback::Callback::Code { code, .. } => {
                let tokens = crate::token::exchange_code(
                    &self.http,
                    &endpoints.token_endpoint,
                    &code,
                    &redirect_uri,
                    &pkce.verifier,
                )
                .await?;
                crate::store::save(&tokens, &self.token_file()?)?;
                self.remember(&tokens);
                Ok(tokens.access_token)
            }
        }
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // #[tokio::test] expands to Runtime::block_on
mod tests {
    use super::{Clock, IdentitySession, Launcher, SessionConfig};
    use crate::error::{IdentityError, Prompt};
    use crate::store::StoredTokens;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct FixedClock(i64);

    impl Clock for FixedClock {
        fn now(&self) -> i64 {
            self.0
        }
    }

    struct FlagLauncher {
        called: Arc<AtomicBool>,
    }

    impl Launcher for FlagLauncher {
        fn launch<'a>(
            &'a self,
            _url: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), IdentityError>> + Send + 'a>> {
            self.called.store(true, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }
    }

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap()
    }

    fn session(
        token_path: std::path::PathBuf,
        now: i64,
        launcher: Arc<FlagLauncher>,
        platform_url: String,
        identity_host: String,
    ) -> IdentitySession {
        IdentitySession::with_config(SessionConfig {
            http: http_client(),
            platform_url,
            identity_host,
            token_path,
            clock: Arc::new(FixedClock(now)),
            launcher,
        })
    }

    async fn serve_idp(
        access: &str,
        refresh: &str,
    ) -> (
        std::net::SocketAddr,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        serve_idp_token_status(access, refresh, 200, "").await
    }

    async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
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

    async fn write_json(stream: &mut tokio::net::TcpStream, body: &str) {
        write_json_status(stream, 200, body).await;
    }

    async fn write_json_status(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
        let reason = if (200..300).contains(&status) {
            "OK"
        } else {
            "Error"
        };
        let header = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes()).await;
        let _ = stream.write_all(body.as_bytes()).await;
        let _ = stream.shutdown().await;
    }

    async fn serve_idp_token_status(
        access: &str,
        refresh: &str,
        token_status: u16,
        token_body: &str,
    ) -> (
        std::net::SocketAddr,
        Arc<AtomicUsize>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let posts = Arc::new(AtomicUsize::new(0));
        let posts_task = posts.clone();
        let well_known = serde_json::json!({
            "idp": {
                "issuer": format!("http://{addr}"),
                "authorization_endpoint": format!("http://{addr}/oauth/authorize"),
                "token_endpoint": format!("http://{addr}/oauth/token"),
            },
            "kas": { "uri": "https://platform.arkavo.net" }
        })
        .to_string();
        let success_body = serde_json::json!({
            "access_token": access,
            "refresh_token": refresh,
            "expires_in": 3600,
        })
        .to_string();
        let token_body = if token_status == 200 {
            success_body
        } else {
            token_body.to_owned()
        };
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let well_known = well_known.clone();
                let token_body = token_body.clone();
                let posts_task = posts_task.clone();
                tokio::spawn(async move {
                    let req = read_request(&mut stream).await;
                    if req.starts_with("POST ") {
                        posts_task.fetch_add(1, Ordering::SeqCst);
                        write_json_status(&mut stream, token_status, &token_body).await;
                    } else {
                        write_json(&mut stream, &well_known).await;
                    }
                });
            }
        });
        (addr, posts, handle)
    }

    #[tokio::test]
    async fn never_prompt_without_tokens_is_login_required() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity_token");
        let called = Arc::new(AtomicBool::new(false));
        let launcher = Arc::new(FlagLauncher {
            called: called.clone(),
        });
        let session = session(
            path,
            0,
            launcher,
            "http://127.0.0.1:1".into(),
            "127.0.0.1".into(),
        );
        let err = session
            .bearer(Prompt::Never)
            .await
            .expect_err("empty store + Never must fail");
        match err {
            IdentityError::LoginRequired(_) => {
                assert!(
                    err.kas_denied_message().contains("run 'arkavo login'"),
                    "{}",
                    err.kas_denied_message()
                );
            }
            other => panic!("expected LoginRequired, got {other:?}"),
        }
        assert!(
            !called.load(Ordering::SeqCst),
            "launcher must not run on Prompt::Never"
        );
    }

    #[tokio::test]
    async fn refresh_before_expiry_uses_injected_clock_and_persists_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity_token");
        crate::store::save(
            &StoredTokens {
                access_token: "old".into(),
                refresh_token: Some("r1".into()),
                expires_at: 1000,
            },
            &path,
        )
        .unwrap();
        let (addr, posts, handle) = serve_idp("new", "r2").await;
        let called = Arc::new(AtomicBool::new(false));
        let launcher = Arc::new(FlagLauncher {
            called: called.clone(),
        });
        let session = session(
            path.clone(),
            950,
            launcher,
            format!("http://{addr}"),
            "127.0.0.1".into(),
        );
        let token = session.bearer(Prompt::Never).await.expect("refresh");
        assert_eq!(token, "new");
        let stored = crate::store::load(&path).unwrap().unwrap();
        assert_eq!(stored.access_token, "new");
        assert_eq!(stored.refresh_token.as_deref(), Some("r2"));
        assert!(
            !called.load(Ordering::SeqCst),
            "refresh must not launch Creator"
        );
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        handle.abort();
    }

    #[tokio::test]
    async fn invalidate_drops_cached_access_but_refresh_still_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity_token");
        crate::store::save(
            &StoredTokens {
                access_token: "old".into(),
                refresh_token: Some("r1".into()),
                expires_at: 1_000_000_000,
            },
            &path,
        )
        .unwrap();
        let (addr, posts, handle) = serve_idp("new", "r2").await;
        let called = Arc::new(AtomicBool::new(false));
        let launcher = Arc::new(FlagLauncher {
            called: called.clone(),
        });
        let session = session(
            path,
            100,
            launcher,
            format!("http://{addr}"),
            "127.0.0.1".into(),
        );
        let first = session.bearer(Prompt::Never).await.unwrap();
        assert_eq!(first, "old");
        assert_eq!(posts.load(Ordering::SeqCst), 0);
        let second = session.bearer(Prompt::Never).await.unwrap();
        assert_eq!(second, "old");
        assert_eq!(posts.load(Ordering::SeqCst), 0);
        session.invalidate();
        let third = session.bearer(Prompt::Never).await.unwrap();
        assert_eq!(third, "new");
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        assert!(!called.load(Ordering::SeqCst));
        handle.abort();
    }

    #[tokio::test]
    async fn dead_refresh_never_prompt_is_login_required_and_file_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity_token");
        crate::store::save(
            &StoredTokens {
                access_token: "old".into(),
                refresh_token: Some("r1".into()),
                expires_at: 1000,
            },
            &path,
        )
        .unwrap();
        let (addr, posts, handle) =
            serve_idp_token_status("new", "r2", 400, r#"{"error":"invalid_grant"}"#).await;
        let called = Arc::new(AtomicBool::new(false));
        let launcher = Arc::new(FlagLauncher {
            called: called.clone(),
        });
        let session = session(
            path.clone(),
            950,
            launcher,
            format!("http://{addr}"),
            "127.0.0.1".into(),
        );
        let err = session
            .bearer(Prompt::Never)
            .await
            .expect_err("dead refresh + Never must fail");
        match err {
            IdentityError::LoginRequired(_) => {
                assert!(
                    err.kas_denied_message().contains("run 'arkavo login'"),
                    "{}",
                    err.kas_denied_message()
                );
            }
            other => panic!("expected LoginRequired, got {other:?}"),
        }
        assert!(!path.exists(), "dead refresh must delete the token file");
        assert!(
            !called.load(Ordering::SeqCst),
            "launcher must not run on Prompt::Never"
        );
        assert_eq!(posts.load(Ordering::SeqCst), 1);
        handle.abort();
    }

    #[tokio::test]
    async fn corrupt_store_never_prompt_is_login_required_and_file_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity_token");
        std::fs::write(&path, b"not-json{{{").unwrap();
        let called = Arc::new(AtomicBool::new(false));
        let launcher = Arc::new(FlagLauncher {
            called: called.clone(),
        });
        let session = session(
            path.clone(),
            0,
            launcher,
            "http://127.0.0.1:1".into(),
            "127.0.0.1".into(),
        );
        let err = session
            .bearer(Prompt::Never)
            .await
            .expect_err("corrupt store + Never must fail");
        match err {
            IdentityError::LoginRequired(_) => {}
            other => panic!("expected LoginRequired, got {other:?}"),
        }
        assert!(!path.exists(), "corrupt store must delete the token file");
        assert!(!called.load(Ordering::SeqCst));
    }
}
