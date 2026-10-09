//! Refuses requests that a web page on another site can send to the gateway.
//!
//! The gateway is an unauthenticated control surface on a well-known port.
//! A page the user visits can open a WebSocket to it: the browser attaches
//! the page's `Origin` but blocks nothing. And a page whose DNS name the
//! attacker points at 127.0.0.1 (DNS rebinding) is same-origin as far as the
//! browser knows, so it can read and post to every route. Two checks close
//! both: the `Host` must name this machine, not a domain, and an `Origin`,
//! when one is sent, must be the gateway's own.

use arkavo_validation::HostValidator;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::IpAddr;
use std::sync::Arc;

/// Why a request was refused. Neither variant echoes the request's headers,
/// which the sender controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The `Host` is missing, unreadable or a name this gateway does not answer to.
    Host,
    /// The `Origin` is unreadable, repeated or another site's.
    Origin,
}

impl Refusal {
    fn message(self) -> &'static str {
        match self {
            Self::Host => "host not allowed",
            Self::Origin => "cross-origin request refused",
        }
    }
}

/// The check for a gateway listening on `bind_ip`.
#[derive(Clone)]
pub struct OriginGuard {
    bind_ip: IpAddr,
    local_names: Arc<HostValidator>,
}

impl OriginGuard {
    pub fn new(bind_ip: IpAddr) -> Self {
        Self {
            bind_ip,
            local_names: Arc::new(HostValidator::new()),
        }
    }

    /// Whether a request with these headers may reach the gateway.
    ///
    /// A request without `Origin` is admitted: browsers send one with every
    /// WebSocket upgrade and every cross-origin request, so its absence means
    /// a non-browser client or a plain navigation.
    pub fn admit(&self, host: Option<&str>, origin: Option<&str>) -> Result<(), Refusal> {
        let host = host
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .ok_or(Refusal::Host)?;
        if !self.host_allowed(host) {
            return Err(Refusal::Host);
        }
        let Some(origin) = origin else {
            return Ok(());
        };
        // An opaque origin ("null") and any non-web scheme have no prefix.
        let authority = origin
            .strip_prefix("http://")
            .or_else(|| origin.strip_prefix("https://"))
            .ok_or(Refusal::Origin)?;
        if authority.eq_ignore_ascii_case(host) {
            Ok(())
        } else {
            Err(Refusal::Origin)
        }
    }

    /// `localhost` and the loopback addresses always; any other address only
    /// when typed as an IP literal this gateway listens on. A literal cannot
    /// be rebound, a DNS name can.
    fn host_allowed(&self, host: &str) -> bool {
        if self.local_names.validate(host).is_ok() {
            return true;
        }
        match host_ip(host) {
            Some(ip) => self.bind_ip.is_unspecified() || ip == self.bind_ip,
            None => false,
        }
    }

    fn admit_headers(&self, headers: &HeaderMap, uri: &Uri) -> Result<(), Refusal> {
        // HTTP/2 carries the host in the request's authority, not a header.
        let host = match headers.get(header::HOST) {
            Some(value) => Some(value.to_str().map_err(|_| Refusal::Host)?),
            None => uri.authority().map(|a| a.as_str()),
        };
        let mut origins = headers.get_all(header::ORIGIN).iter();
        let origin = match (origins.next(), origins.next()) {
            (None, _) => None,
            (Some(value), None) => Some(value.to_str().map_err(|_| Refusal::Origin)?),
            // Two Origins cannot both be the browser's; refuse rather than pick.
            (Some(_), Some(_)) => return Err(Refusal::Origin),
        };
        self.admit(host, origin)
    }
}

/// The IP address a `Host` value names, without its port, or `None` for a name.
fn host_ip(host: &str) -> Option<IpAddr> {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next()?.parse().ok();
    }
    let name = host.rsplit_once(':').map_or(host, |(name, _)| name);
    name.parse().ok()
}

/// Middleware applying [`OriginGuard`] to every route, the WebSocket upgrades
/// included.
pub async fn guard(State(guard): State<OriginGuard>, request: Request, next: Next) -> Response {
    match guard.admit_headers(request.headers(), request.uri()) {
        Ok(()) => next.run(request).await,
        Err(refusal) => {
            tracing::warn!(?refusal, path = %request.uri().path(), "AG-UI: refused request");
            (StatusCode::FORBIDDEN, refusal.message()).into_response()
        }
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // #[tokio::test] expands to Runtime::block_on
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use std::net::{Ipv4Addr, Ipv6Addr};

    fn loopback() -> OriginGuard {
        OriginGuard::new(IpAddr::V4(Ipv4Addr::LOCALHOST))
    }

    #[spec("AGUI-021")]
    #[test]
    fn the_gateways_own_pages_are_admitted() {
        let guard = loopback();
        for host in [
            "127.0.0.1:7700",
            "localhost:7700",
            "LOCALHOST:7700",
            "[::1]:7700",
        ] {
            assert_eq!(guard.admit(Some(host), None), Ok(()), "{host}");
            let origin = format!("http://{host}");
            assert_eq!(guard.admit(Some(host), Some(&origin)), Ok(()), "{origin}");
        }
    }

    /// Regression: a page on another site opened `/ws` on a running gateway
    /// (2026-10-04) and could dispatch tasks to agents through it.
    #[spec("AGUI-021")]
    #[test]
    fn another_sites_origin_is_refused() {
        let guard = loopback();
        for origin in [
            "https://evil.example",
            "http://evil.example:7700",
            "http://127.0.0.1:8080",
            "http://localhost",
            "null",
            "file://",
        ] {
            assert_eq!(
                guard.admit(Some("127.0.0.1:7700"), Some(origin)),
                Err(Refusal::Origin),
                "{origin}"
            );
        }
    }

    /// A rebound name is same-origin to the browser, so only the Host check
    /// stops it.
    #[spec("AGUI-021")]
    #[test]
    fn a_dns_name_is_refused_even_when_origin_matches() {
        let guard = loopback();
        assert_eq!(
            guard.admit(Some("evil.example:7700"), Some("http://evil.example:7700")),
            Err(Refusal::Host)
        );
        assert_eq!(
            guard.admit(Some("evil.example:7700"), None),
            Err(Refusal::Host)
        );
        assert_eq!(
            guard.admit(Some("localhost.evil.example"), None),
            Err(Refusal::Host)
        );
    }

    #[spec("AGUI-021")]
    #[test]
    fn a_missing_host_is_refused() {
        let guard = loopback();
        assert_eq!(guard.admit(None, None), Err(Refusal::Host));
        assert_eq!(guard.admit(Some("  "), None), Err(Refusal::Host));
    }

    #[spec("AGUI-021")]
    #[test]
    fn an_address_literal_is_admitted_only_where_the_gateway_listens() {
        let lan = "192.168.1.5:7700";
        assert_eq!(loopback().admit(Some(lan), None), Err(Refusal::Host));

        let everywhere = OriginGuard::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        assert_eq!(
            everywhere.admit(Some(lan), Some("http://192.168.1.5:7700")),
            Ok(())
        );
        assert_eq!(everywhere.admit(Some("[fe80::1]:7700"), None), Ok(()));
        assert_eq!(
            everywhere.admit(Some("box.local:7700"), None),
            Err(Refusal::Host)
        );

        let one = OriginGuard::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)));
        assert_eq!(one.admit(Some("192.168.1.10:7700"), None), Ok(()));
        assert_eq!(one.admit(Some(lan), None), Err(Refusal::Host));

        let v6 = OriginGuard::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
        assert_eq!(v6.admit(Some("[2001:db8::7]:7700"), None), Ok(()));
    }

    #[spec("AGUI-021")]
    #[test]
    fn unreadable_or_repeated_headers_are_refused() {
        let guard = loopback();
        let uri: Uri = "/ws".parse().unwrap();

        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:7700".parse().unwrap());
        headers.append(header::ORIGIN, "http://127.0.0.1:7700".parse().unwrap());
        headers.append(header::ORIGIN, "https://evil.example".parse().unwrap());
        assert_eq!(guard.admit_headers(&headers, &uri), Err(Refusal::Origin));

        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "127.0.0.1:7700".parse().unwrap());
        headers.insert(
            header::ORIGIN,
            axum::http::HeaderValue::from_bytes(b"http://\xff").unwrap(),
        );
        assert_eq!(guard.admit_headers(&headers, &uri), Err(Refusal::Origin));
    }

    #[spec("AGUI-021")]
    #[test]
    fn an_http2_authority_stands_in_for_host() {
        let guard = loopback();
        let uri: Uri = "http://127.0.0.1:7700/ws".parse().unwrap();
        assert_eq!(guard.admit_headers(&HeaderMap::new(), &uri), Ok(()));
        let rebound: Uri = "http://evil.example:7700/ws".parse().unwrap();
        assert_eq!(
            guard.admit_headers(&HeaderMap::new(), &rebound),
            Err(Refusal::Host)
        );
    }

    /// Sends a WebSocket upgrade over a real socket and returns the status line.
    async fn upgrade_status(addr: std::net::SocketAddr, host: &str, origin: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let request = format!(
            "GET /ws HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\
             Origin: {origin}\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 256];
        let n = stream.read(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf[..n])
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    }

    /// Regression, end to end: the forged-Origin upgrade that got
    /// `101 Switching Protocols` now gets 403 before any handler runs.
    #[spec("AGUI-021")]
    #[tokio::test]
    async fn a_forged_origin_upgrade_never_reaches_the_websocket() {
        use axum::extract::ws::WebSocketUpgrade;
        use axum::routing::get;

        let app = axum::Router::new()
            .route(
                "/ws",
                get(|ws: WebSocketUpgrade| async move { ws.on_upgrade(|_| async {}) }),
            )
            .layer(axum::middleware::from_fn_with_state(loopback(), guard));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let host = format!("127.0.0.1:{}", addr.port());
        let forged = upgrade_status(addr, &host, "https://evil.example").await;
        assert!(forged.contains(" 403 "), "{forged}");

        let own = upgrade_status(addr, &host, &format!("http://{host}")).await;
        assert!(own.contains(" 101 "), "{own}");
    }
}
