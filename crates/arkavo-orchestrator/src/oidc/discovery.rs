//! OIDC Discovery endpoint.

use super::provider::OidcProvider;
use super::types::DiscoveryDocument;
use axum::Json;
use axum::extract::State;
use std::sync::Arc;

/// Handle discovery request (GET /.well-known/openid-configuration).
pub async fn discovery(State(provider): State<Arc<OidcProvider>>) -> Json<DiscoveryDocument> {
    let issuer = provider.issuer().to_string();

    Json(DiscoveryDocument {
        issuer: issuer.clone(),
        token_endpoint: format!("{issuer}/token"),
        jwks_uri: format!("{issuer}/jwks"),
        response_types_supported: vec!["token".to_string()],
        subject_types_supported: vec!["public".to_string()],
        id_token_signing_alg_values_supported: vec!["RS256".to_string()],
        grant_types_supported: vec!["client_credentials".to_string()],
        token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
    })
}
