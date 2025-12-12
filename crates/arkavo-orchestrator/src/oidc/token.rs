//! Token endpoint handler.

use super::provider::OidcProvider;
use super::types::{TokenRequest, TokenResponse};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Form;
use axum::Json;
use std::sync::Arc;

/// Error response for token endpoint.
#[derive(serde::Serialize)]
struct TokenError {
    error: String,
    error_description: String,
}

impl IntoResponse for TokenError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

/// Handle token request (POST /token).
pub async fn token(
    State(provider): State<Arc<OidcProvider>>,
    Form(req): Form<TokenRequest>,
) -> Result<Json<TokenResponse>, Response> {
    // Validate grant type
    if req.grant_type != "client_credentials" {
        return Err(TokenError {
            error: "unsupported_grant_type".to_string(),
            error_description: "Only client_credentials grant is supported".to_string(),
        }
        .into_response());
    }

    // Validate client credentials
    if !provider.validate_client(&req.client_id, req.client_secret.as_deref()) {
        return Err(TokenError {
            error: "invalid_client".to_string(),
            error_description: "Invalid client credentials".to_string(),
        }
        .into_response());
    }

    // Issue token
    let access_token = provider
        .issue_token(&req.client_id, req.scope.as_deref())
        .map_err(|e| {
            TokenError {
                error: "server_error".to_string(),
                error_description: e,
            }
            .into_response()
        })?;

    Ok(Json(TokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: provider.token_expiry_secs(),
        scope: req.scope,
    }))
}
