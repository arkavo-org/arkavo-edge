use crate::error::{Error, Result};
use crate::types::GitHubEvent;
use axum::{
    Router,
    body::Bytes,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower_http::cors::CorsLayer;
use tracing::{error, info, warn};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
pub struct WebhookServer {
    secret: String,
    event_tx: mpsc::UnboundedSender<GitHubEvent>,
}

impl WebhookServer {
    pub fn new(secret: String) -> (Self, mpsc::UnboundedReceiver<GitHubEvent>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        (Self { secret, event_tx }, event_rx)
    }

    pub fn router(self) -> Router {
        let state = Arc::new(self);

        Router::new()
            .route("/webhook", post(handle_webhook))
            .route("/health", axum::routing::get(health_check))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                verify_signature,
            ))
            .layer(CorsLayer::permissive())
            .with_state(state)
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

async fn verify_signature(
    State(server): State<Arc<WebhookServer>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    if path == "/health" {
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
    let event_type = headers
        .get("X-GitHub-Event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    info!(event_type, "Processing webhook");

    let event: GitHubEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            error!("Failed to parse webhook event: {e}");
            return (StatusCode::BAD_REQUEST, "Invalid event format").into_response();
        }
    };

    if let Err(e) = server.process_event(event) {
        error!("Failed to process event: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to process event").into_response();
    }

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
        let (server, _rx) = WebhookServer::new(secret.to_string());

        let payload = b"test payload";
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(payload);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        let result = server.verify_signature(&signature, payload);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_signature() {
        let (server, _rx) = WebhookServer::new("test-secret".to_string());

        let payload = b"test payload";
        let signature = "sha256=invalid";

        let result = server.verify_signature(signature, payload);
        assert!(result.is_err());
    }
}
