use crate::error::{Error, Result};
use crate::types::GitHubEvent;
use arkavo_observability::metrics::MetricsCollector;
use arkavo_protocol::rate_limit::{IpRateLimiter, RateLimitConfig};
use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;
use tracing::{error, info, warn};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct WebhookServer {
    secret: String,
    event_tx: mpsc::UnboundedSender<GitHubEvent>,
    metrics: Arc<MetricsCollector>,
    rate_limiter: Arc<IpRateLimiter>,
}

impl WebhookServer {
    pub fn new(
        secret: String,
        rate_limit_config: RateLimitConfig,
    ) -> (Self, mpsc::UnboundedReceiver<GitHubEvent>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let metrics = Arc::new(MetricsCollector::new());
        let rate_limiter = Arc::new(IpRateLimiter::new(rate_limit_config));

        (
            Self {
                secret,
                event_tx,
                metrics,
                rate_limiter,
            },
            event_rx,
        )
    }

    pub fn router(self) -> Router {
        let state = Arc::new(self);

        Router::new()
            .route("/webhook", post(handle_webhook))
            .route("/health", axum::routing::get(health_check))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                rate_limit_middleware,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                verify_signature,
            ))
            .layer(CorsLayer::permissive())
            .with_state(state)
    }

    /// Create router with OIDC endpoints merged.
    ///
    /// OIDC endpoints (/.well-known/*, /token, /jwks) bypass signature verification
    /// and rate limiting, as they use their own authentication (client credentials).
    pub fn router_with_oidc(self, oidc_provider: Arc<crate::oidc::OidcProvider>) -> Router {
        let state = Arc::new(self);

        // Create OIDC router with its state finalized (becomes Router<()>)
        let oidc_routes = crate::oidc::router(oidc_provider);

        // Webhook routes with WebhookServer state
        let webhook_routes = Router::new()
            .route("/webhook", post(handle_webhook))
            .route("/health", axum::routing::get(health_check))
            .with_state(state.clone());

        // Merge both routers (both are now Router<()>)
        webhook_routes
            .merge(oidc_routes)
            .layer(middleware::from_fn_with_state(
                state.clone(),
                rate_limit_middleware,
            ))
            .layer(middleware::from_fn_with_state(state, verify_signature))
            .layer(CorsLayer::permissive())
    }

    fn verify_signature(&self, signature: &str, payload: &[u8]) -> Result<()> {
        let signature = signature
            .strip_prefix("sha256=")
            .ok_or(Error::InvalidSignature)?;

        let expected = hex::decode(signature).map_err(|_| Error::InvalidSignature)?;

        let mut mac =
            HmacSha256::new_from_slice(self.secret.as_bytes()).map_err(|_| Error::MissingSecret)?;

        mac.update(payload);

        mac.verify_slice(&expected)
            .map_err(|_| Error::InvalidSignature)?;

        Ok(())
    }

    #[allow(clippy::unused_async)]
    fn process_event(&self, event: GitHubEvent) -> Result<()> {
        info!(
            event_type = event.event_type(),
            "Received GitHub webhook event"
        );

        self.event_tx
            .send(event)
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to send event: {e}")))?;

        Ok(())
    }
}

/// Check if a path is an OIDC endpoint that should bypass webhook middleware.
fn is_oidc_path(path: &str) -> bool {
    path.starts_with("/.well-known") || path == "/token" || path == "/jwks"
}

async fn rate_limit_middleware(
    State(server): State<Arc<WebhookServer>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    // Bypass rate limiting for health and OIDC endpoints
    if path == "/health" || is_oidc_path(path) {
        return next.run(request).await;
    }

    if let Err(e) = server.rate_limiter.check_rate_limit(addr.ip()) {
        warn!(ip = %addr.ip(), error = %e, "Rate limit exceeded");
        server.metrics.record_error("rate_limit");
        return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded").into_response();
    }

    next.run(request).await
}

async fn verify_signature(
    State(server): State<Arc<WebhookServer>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    // Bypass signature verification for health and OIDC endpoints
    if path == "/health" || is_oidc_path(path) {
        return next.run(request).await;
    }

    let signature = match headers.get("X-Hub-Signature-256") {
        Some(sig) => match sig.to_str() {
            Ok(s) => s,
            Err(_) => {
                warn!("Invalid signature header encoding");
                return (StatusCode::BAD_REQUEST, "Invalid signature header").into_response();
            }
        },
        None => {
            warn!("Missing X-Hub-Signature-256 header");
            return (StatusCode::UNAUTHORIZED, "Missing signature").into_response();
        }
    };

    let (parts, body) = request.into_parts();
    let bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(e) => {
            error!("Failed to read request body: {e}");
            return (StatusCode::BAD_REQUEST, "Invalid request body").into_response();
        }
    };

    if let Err(e) = server.verify_signature(signature, &bytes) {
        warn!("Signature verification failed: {e}");
        return (StatusCode::UNAUTHORIZED, "Invalid signature").into_response();
    }

    let request = Request::from_parts(parts, axum::body::Body::from(bytes));
    next.run(request).await
}

async fn handle_webhook(
    State(server): State<Arc<WebhookServer>>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let start = Instant::now();
    let body_size = body.len();

    let event_type = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    info!(event_type, body_size, "Processing webhook");

    server.metrics.record_message_sent(body_size);

    let event: GitHubEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            error!("Failed to parse webhook event: {e}");
            server.metrics.record_error("parse_error");
            return (StatusCode::BAD_REQUEST, "Invalid event format").into_response();
        }
    };

    if let Err(e) = server.process_event(event) {
        error!("Failed to process event: {e}");
        server.metrics.record_error("processing_error");
        let elapsed = start.elapsed().as_millis() as u64;
        server.metrics.record_response_time(elapsed);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to process event").into_response();
    }

    let elapsed = start.elapsed().as_millis() as u64;
    server.metrics.record_response_time(elapsed);
    info!(elapsed_ms = elapsed, "Webhook processed successfully");

    (StatusCode::OK, "Event received").into_response()
}

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature_verification() {
        let secret = "test-secret";
        let config = RateLimitConfig::default();
        let (server, _rx) = WebhookServer::new(secret.to_string(), config);

        let payload = b"test payload";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let result = server.verify_signature(&signature, payload);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_signature() {
        let config = RateLimitConfig::default();
        let (server, _rx) = WebhookServer::new("test-secret".to_string(), config);

        let payload = b"test payload";
        let signature = "sha256=invalid";

        let result = server.verify_signature(signature, payload);
        assert!(result.is_err());
    }
}
