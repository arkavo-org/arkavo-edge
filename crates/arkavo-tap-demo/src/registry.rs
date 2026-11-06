//! The development registry: a local trust authority for test keys.
//!
//! Registering here is not a Visa registration or any production onboarding. Writes are
//! accepted only from loopback, its control plane; reads serve key documents to verifiers.

use anyhow::Context;
use arkavo_tap::keys::key_id_is_valid;
use arkavo_tap::{Clock, KeyDocument, Purpose, TrustAuthority};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row as _, SqlitePool};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

const MAX_VALIDITY_SECONDS: i64 = 30 * 86_400;

/// A public key to register.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Registration {
    pub key_id: String,
    /// The Ed25519 public key, base64url.
    pub x: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    pub purposes: Vec<String>,
    pub validity_seconds: i64,
}

/// The registry's state.
pub struct Registry {
    pool: SqlitePool,
    authority: TrustAuthority,
    clock: Arc<dyn Clock>,
    max_age: i64,
}

impl Registry {
    pub async fn open(
        path: &std::path::Path,
        authority: TrustAuthority,
        clock: Arc<dyn Clock>,
        max_age: i64,
    ) -> anyhow::Result<Self> {
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .with_context(|| format!("opening {}", path.display()))?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS keys (
                key_id TEXT PRIMARY KEY NOT NULL,
                x TEXT NOT NULL,
                provider TEXT NOT NULL,
                runtime TEXT,
                purposes TEXT NOT NULL,
                status TEXT NOT NULL,
                not_before INTEGER NOT NULL,
                not_after INTEGER NOT NULL
            )",
        )
        .execute(&pool)
        .await?;
        Ok(Registry {
            pool,
            authority,
            clock,
            max_age,
        })
    }

    async fn document(&self, key_id: &str) -> Result<Option<KeyDocument>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT x, provider, runtime, purposes, status, not_before, not_after FROM keys WHERE key_id = ?",
        )
        .bind(key_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| KeyDocument {
            authority: self.authority.name.clone(),
            environment: self.authority.environment.name().to_owned(),
            key_id: key_id.to_owned(),
            kty: "OKP".into(),
            crv: "Ed25519".into(),
            x: r.get("x"),
            status: r.get("status"),
            not_before: r.get("not_before"),
            not_after: r.get("not_after"),
            provider: r.get("provider"),
            runtime: r.get("runtime"),
            purposes: r
                .get::<String, _>("purposes")
                .split(',')
                .map(str::to_owned)
                .collect(),
            max_age_seconds: self.max_age,
        }))
    }

    async fn register(&self, reg: &Registration) -> Result<KeyDocument, (StatusCode, String)> {
        let bad = |why: &str| (StatusCode::BAD_REQUEST, why.to_owned());
        if !key_id_is_valid(&reg.key_id) {
            return Err(bad("keyId is not a valid key identifier"));
        }
        let x = URL_SAFE_NO_PAD
            .decode(&reg.x)
            .map_err(|_| bad("x is not base64url"))?;
        if x.len() != 32 {
            return Err(bad("x must be a 32-byte Ed25519 public key"));
        }
        if reg.purposes.is_empty() || reg.purposes.iter().any(|p| Purpose::from_tag(p).is_none()) {
            return Err(bad("purposes must be TAP tags"));
        }
        if !(1..=MAX_VALIDITY_SECONDS).contains(&reg.validity_seconds) {
            return Err(bad("validitySeconds must be 1 second to 30 days"));
        }
        let now = self.clock.now();
        let inserted = sqlx::query(
            "INSERT INTO keys (key_id, x, provider, runtime, purposes, status, not_before, not_after)
             VALUES (?, ?, ?, ?, ?, 'active', ?, ?)",
        )
        .bind(&reg.key_id)
        .bind(&reg.x)
        .bind(&reg.provider)
        .bind(&reg.runtime)
        .bind(reg.purposes.join(","))
        .bind(now)
        .bind(now + reg.validity_seconds)
        .execute(&self.pool)
        .await;
        match inserted {
            Ok(_) => {}
            Err(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                return Err((
                    StatusCode::CONFLICT,
                    "the key ID is already registered".into(),
                ));
            }
            Err(e) => return Err((StatusCode::SERVICE_UNAVAILABLE, e.to_string())),
        }
        self.document(&reg.key_id)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "registration not readable".into(),
                )
            })
    }

    async fn revoke(&self, key_id: &str) -> Result<KeyDocument, (StatusCode, String)> {
        sqlx::query("UPDATE keys SET status = 'revoked' WHERE key_id = ?")
            .bind(key_id)
            .execute(&self.pool)
            .await
            .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
        match self.document(key_id).await {
            Ok(Some(doc)) => Ok(doc),
            Ok(None) => Err((StatusCode::NOT_FOUND, "no such key".into())),
            Err(e) => Err((StatusCode::SERVICE_UNAVAILABLE, e.to_string())),
        }
    }
}

/// A refusal unless `peer` is on loopback, where the control plane lives.
fn control_plane(peer: SocketAddr) -> Option<Response> {
    if peer.ip().is_loopback() {
        None
    } else {
        Some(
            (
                StatusCode::FORBIDDEN,
                "the registry control plane is loopback-only",
            )
                .into_response(),
        )
    }
}

fn answer(result: Result<KeyDocument, (StatusCode, String)>, created: bool) -> Response {
    match result {
        Ok(doc) if created => (StatusCode::CREATED, Json(doc)).into_response(),
        Ok(doc) => Json(doc).into_response(),
        Err((status, why)) => (status, Json(serde_json::json!({ "error": why }))).into_response(),
    }
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn register(
    State(registry): State<Arc<Registry>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(reg): Json<Registration>,
) -> Response {
    if let Some(refused) = control_plane(peer) {
        return refused;
    }
    answer(registry.register(&reg).await, true)
}

async fn revoke(
    State(registry): State<Arc<Registry>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(key_id): Path<String>,
) -> Response {
    if let Some(refused) = control_plane(peer) {
        return refused;
    }
    answer(registry.revoke(&key_id).await, false)
}

async fn key(State(registry): State<Arc<Registry>>, Path(key_id): Path<String>) -> Response {
    match registry.document(&key_id).await {
        Ok(Some(doc)) => Json(doc).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// The registry's routes.
pub fn router(registry: Arc<Registry>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/dev/v1/agents", post(register))
        .route("/dev/v1/agents/:key_id/revoke", post(revoke))
        .route("/dev/v1/keys/:key_id", get(key))
        .with_state(registry)
}

/// Serves the registry on `listener` until `shutdown` completes, then closes every
/// connection, idle keep-alive ones included, so a stopped registry really is unreachable.
pub async fn serve(
    listener: tokio::net::TcpListener,
    registry: Arc<Registry>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    axum::serve(
        listener,
        router(registry).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .context("serving the registry")
}
