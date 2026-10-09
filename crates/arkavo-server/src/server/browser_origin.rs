//! Refuses requests a web page sends to the agent's RPC listener.
//!
//! An agent serves no web pages, so a request that carries an `Origin` comes
//! from a page in a browser: browsers send one with every WebSocket upgrade
//! and every POST, and enforce nothing on a WebSocket. The listener
//! authenticates no caller, so before this any page the user visited could
//! drive an agent on loopback. Agents, the UI gateway and other clients send
//! no `Origin` and are unaffected.

use futures::future::BoxFuture;
use jsonrpsee::server::{HttpRequest, HttpResponse, http::response};
use std::task::{Context, Poll};
use tower::{Layer, Service};

/// Layer that answers any request carrying an `Origin` header with 403.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct RefuseBrowserOriginLayer;

impl<S> Layer<S> for RefuseBrowserOriginLayer {
    type Service = RefuseBrowserOrigin<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RefuseBrowserOrigin { inner }
    }
}

#[derive(Debug, Clone)]
pub(super) struct RefuseBrowserOrigin<S> {
    inner: S,
}

impl<S, B> Service<HttpRequest<B>> for RefuseBrowserOrigin<S>
where
    S: Service<HttpRequest<B>, Response = HttpResponse>,
    S::Error: Send + 'static,
    S::Future: Send + 'static,
{
    type Response = HttpResponse;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<HttpResponse, S::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: HttpRequest<B>) -> Self::Future {
        if request.headers().contains_key("origin") {
            tracing::warn!(path = %request.uri().path(), "Refused an RPC request from a web page");
            return Box::pin(std::future::ready(Ok(response::denied())));
        }
        Box::pin(self.inner.call(request))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // #[tokio::test] expands to Runtime::block_on
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use jsonrpsee::RpcModule;
    use jsonrpsee::server::ServerBuilder;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn start() -> (SocketAddr, jsonrpsee::server::ServerHandle) {
        let server = ServerBuilder::default()
            .set_http_middleware(tower::ServiceBuilder::new().layer(RefuseBrowserOriginLayer))
            .build("127.0.0.1:0")
            .await
            .unwrap();
        let addr = server.local_addr().unwrap();
        let mut module = RpcModule::new(());
        module.register_method("health", |_, _, _| "ok").unwrap();
        (addr, server.start(module))
    }

    /// Sends a raw request and returns its status line.
    async fn status(addr: SocketAddr, request: String) -> String {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut buf = vec![0u8; 512];
        let n = stream.read(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf[..n])
            .lines()
            .next()
            .unwrap_or_default()
            .to_string()
    }

    fn upgrade(addr: SocketAddr, origin: Option<&str>) -> String {
        let origin = origin
            .map(|o| format!("Origin: {o}\r\n"))
            .unwrap_or_default();
        format!(
            "GET / HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{origin}\r\n"
        )
    }

    fn post(addr: SocketAddr, origin: Option<&str>) -> String {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"health"}"#;
        let origin = origin
            .map(|o| format!("Origin: {o}\r\n"))
            .unwrap_or_default();
        format!(
            "POST / HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n{origin}Connection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// Regression: a WebSocket upgrade with `Origin: https://evil.example`
    /// to an agent on loopback got `101 Switching Protocols` (2026-10-02).
    #[spec("INGRESS-001")]
    #[tokio::test]
    async fn a_web_pages_upgrade_is_refused() {
        let (addr, handle) = start().await;
        let forged = status(addr, upgrade(addr, Some("https://evil.example"))).await;
        assert!(forged.contains(" 403 "), "{forged}");
        let local_page = status(addr, upgrade(addr, Some(&format!("http://{addr}")))).await;
        assert!(local_page.contains(" 403 "), "{local_page}");
        handle.stop().unwrap();
    }

    #[spec("INGRESS-001")]
    #[tokio::test]
    async fn a_web_pages_post_is_refused() {
        let (addr, handle) = start().await;
        let forged = status(addr, post(addr, Some("https://evil.example"))).await;
        assert!(forged.contains(" 403 "), "{forged}");
        handle.stop().unwrap();
    }

    #[spec("INGRESS-001")]
    #[tokio::test]
    async fn clients_without_origin_are_unaffected() {
        let (addr, handle) = start().await;
        let ws = status(addr, upgrade(addr, None)).await;
        assert!(ws.contains(" 101 "), "{ws}");
        let http = status(addr, post(addr, None)).await;
        assert!(http.contains(" 200 "), "{http}");
        handle.stop().unwrap();
    }
}
