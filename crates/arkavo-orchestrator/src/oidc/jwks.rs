//! JWKS endpoint for public key retrieval.

use super::provider::OidcProvider;
use super::types::JsonWebKeySet;
use axum::extract::State;
use axum::Json;
use std::sync::Arc;

/// Handle JWKS request (GET /jwks).
pub async fn jwks(State(provider): State<Arc<OidcProvider>>) -> Json<JsonWebKeySet> {
    Json(provider.jwks())
}
