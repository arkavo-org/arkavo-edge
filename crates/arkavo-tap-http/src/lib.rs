//! Axum middleware that admits Trusted Agent Protocol requests before merchant code runs.
//!
//! The gate runs in the merchant process, before routing changes anything. For a protected
//! route it bounds the content, verifies the request against the configured external
//! origin, and attaches the [`VerifiedAgent`] as a request extension: the only place
//! verified context lives. Extensions never come from the wire, and header fields that look
//! like verification results (`x-tap-*`, `x-agent-*`, `x-verified-*`) are stripped from
//! every request, so an `X-Agent-Verified: true` sent by a client establishes nothing.
//! Requests to other routes pass through anonymously, with no verified status.

mod route;

pub use route::{PathPattern, ProtectedRoute};

use arkavo_tap::{
    Clock, DecisionRecord, ExternalOrigin, RejectKind, Rejection, ReplayStore, RequestContext,
    RoutePolicy, TapVerifier, TrustedKeyResolver, VerifiedAgent,
};
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

/// The response header that names the decision record for a request.
pub const CORRELATION_HEADER: &str = "x-correlation-id";

const SPOOFABLE_PREFIXES: [&str; 3] = ["x-tap-", "x-agent-", "x-verified-"];

/// The decision record ID for this request, available to handlers as an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrelationId(pub String);

/// Gate settings.
#[derive(Debug, Clone)]
pub struct GateConfig {
    /// The merchant's external origin, from configuration. Never from `Host` or forwarding
    /// headers.
    pub origin: ExternalOrigin,
    pub max_body_bytes: usize,
    /// Verifications started per second, across all protected routes, before 429.
    pub max_verifications_per_second: u32,
}

/// The verifier, the protected routes and the limits in front of them.
pub struct TapGate<R, S, C> {
    verifier: TapVerifier<R, S, C>,
    config: GateConfig,
    routes: Vec<ProtectedRoute>,
    started: Instant,
    window: Mutex<(u64, u32)>,
}

impl<R, S, C> TapGate<R, S, C>
where
    R: TrustedKeyResolver,
    S: ReplayStore,
    C: Clock,
{
    pub fn new(
        verifier: TapVerifier<R, S, C>,
        config: GateConfig,
        routes: Vec<ProtectedRoute>,
    ) -> Self {
        TapGate {
            verifier,
            config,
            routes,
            started: Instant::now(),
            window: Mutex::new((0, 0)),
        }
    }

    /// The policy for a request. A protected path is protected for every method: a `HEAD`
    /// to a `GET` route, or any method the router might also accept, must verify too, so
    /// nothing reaches a protected handler unverified.
    fn policy_for(&self, request: &Request) -> Option<&RoutePolicy> {
        let path = request.uri().path();
        self.routes
            .iter()
            .find(|r| r.method == request.method() && r.path.matches(path))
            .or_else(|| self.routes.iter().find(|r| r.path.matches(path)))
            .map(|r| &r.policy)
    }

    async fn within_rate(&self) -> bool {
        let second = self.started.elapsed().as_secs();
        let mut window = self.window.lock().await;
        if window.0 != second {
            *window = (second, 0);
        }
        window.1 = window.1.saturating_add(1);
        window.1 <= self.config.max_verifications_per_second
    }

    /// Admits `request` or refuses it. Refusals before the verifier runs are recorded here;
    /// the verifier records its own.
    async fn admit(
        &self,
        request: Request,
        policy: &RoutePolicy,
        id: &str,
    ) -> Result<Request, Rejection> {
        let (mut parts, content) = match self.bounded(request).await {
            Ok(bounded) => bounded,
            Err(rejection) => {
                self.record_early(&rejection, policy, id);
                return Err(rejection);
            }
        };
        let target = parts.uri.path_and_query().map_or("/", |p| p.as_str());
        let ctx = match RequestContext::new(
            self.config.origin.clone(),
            parts.method.as_str(),
            target,
            fields(&parts.headers),
        ) {
            Ok(ctx) => ctx,
            Err(rejection) => {
                self.record_early(&rejection, policy, id);
                return Err(rejection);
            }
        };
        let agent = self.verifier.verify(&ctx, &content, policy, id).await?;
        parts.extensions.insert(agent);
        Ok(Request::from_parts(parts, Body::from(content)))
    }

    async fn bounded(
        &self,
        request: Request,
    ) -> Result<(axum::http::request::Parts, axum::body::Bytes), Rejection> {
        if !self.within_rate().await {
            return Err(Rejection::new(
                RejectKind::RateLimited,
                "verification rate exceeded",
            ));
        }
        let (parts, body) = request.into_parts();
        let declared = parts
            .headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok());
        if declared.is_some_and(|n| n > self.config.max_body_bytes) {
            return Err(Rejection::new(
                RejectKind::ContentTooLarge,
                "declared content exceeds the limit",
            ));
        }
        let content = to_bytes(body, self.config.max_body_bytes)
            .await
            .map_err(|e| {
                Rejection::new(
                    RejectKind::ContentTooLarge,
                    format!("content over the limit or unreadable: {e}"),
                )
            })?;
        Ok((parts, content))
    }

    fn record_early(&self, rejection: &Rejection, policy: &RoutePolicy, id: &str) {
        DecisionRecord {
            decision_id: id.to_owned(),
            profile: policy.profile,
            accepted: false,
            result: rejection.kind.code().to_owned(),
            reason: rejection.detail.clone(),
            key_id: None,
            key_thumbprint: None,
            authority: self.verifier.config().authority.to_string(),
            elapsed_micros: 0,
        }
        .emit();
    }
}

/// Header fields as text. A value that is not visible ASCII cannot be a covered component
/// under any supported profile, so it is left out rather than refused.
fn fields(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .filter_map(|(n, v)| {
            v.to_str()
                .ok()
                .map(|v| (n.as_str().to_owned(), v.to_owned()))
        })
        .collect()
}

fn strip_spoofable(headers: &mut HeaderMap) {
    let spoofed: Vec<HeaderName> = headers
        .keys()
        .filter(|n| SPOOFABLE_PREFIXES.iter().any(|p| n.as_str().starts_with(p)))
        .cloned()
        .collect();
    for name in spoofed {
        headers.remove(name);
    }
}

fn rejection_response(rejection: &Rejection, id: &str) -> Response {
    let status = StatusCode::from_u16(rejection.kind.status()).unwrap_or(StatusCode::FORBIDDEN);
    let body = serde_json::json!({ "error": rejection.kind.code(), "correlationId": id });
    (status, axum::Json(body)).into_response()
}

/// The middleware. Install with `axum::middleware::from_fn_with_state(gate, guard)`.
pub async fn guard<R, S, C>(
    State(gate): State<Arc<TapGate<R, S, C>>>,
    mut request: Request,
    next: Next,
) -> Response
where
    R: TrustedKeyResolver + 'static,
    S: ReplayStore + 'static,
    C: Clock + 'static,
{
    let id = uuid::Uuid::new_v4().to_string();
    strip_spoofable(request.headers_mut());
    request.extensions_mut().remove::<VerifiedAgent>();
    request.extensions_mut().insert(CorrelationId(id.clone()));
    let policy = gate.policy_for(&request).cloned();
    let mut response = match policy {
        None => next.run(request).await,
        Some(policy) => match gate.admit(request, &policy, &id).await {
            Ok(admitted) => next.run(admitted).await,
            Err(rejection) => rejection_response(&rejection, &id),
        },
    };
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert(CORRELATION_HEADER, value);
    }
    response
}
