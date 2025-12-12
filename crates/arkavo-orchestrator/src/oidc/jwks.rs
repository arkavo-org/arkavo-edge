//! JWKS endpoint for public key retrieval.

use super::provider::OidcProvider;
use super::types::JsonWebKeySet;
use axum::Json;
use axum::extract::State;
use std::sync::Arc;

/// Handle JWKS request (GET /jwks).
pub async fn jwks(State(provider): State<Arc<OidcProvider>>) -> Json<JsonWebKeySet> {
    Json(provider.jwks())
}
