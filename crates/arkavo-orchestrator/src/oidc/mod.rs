//! OIDC Provider for orchestrator.
//!
//! Provides minimal OIDC endpoints to replace Keycloak for local OpenTDF.

mod discovery;
mod jwks;
mod provider;
mod token;
mod types;

use axum::{Router, routing::get, routing::post};
use std::sync::Arc;

pub use discovery::discovery;
pub use jwks::jwks;
pub use provider::OidcProvider;
pub use token::token;
pub use types::{ClientRegistration, DiscoveryDocument, TokenRequest, TokenResponse};

/// Create OIDC router with the given provider.
pub fn router(provider: Arc<OidcProvider>) -> Router {
    Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/token", post(token))
        .route("/jwks", get(jwks))
        .with_state(provider)
}
