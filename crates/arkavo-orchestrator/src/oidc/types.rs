//! OIDC types for token requests and responses.

use serde::{Deserialize, Serialize};

/// OAuth client registration.
#[derive(Debug, Clone)]
pub struct ClientRegistration {
    pub client_id: String,
    pub client_secret: String,
    pub scopes: Vec<String>,
}

/// Token request (application/x-www-form-urlencoded).
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub scope: Option<String>,
}

/// OAuth 2.0 token response.
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// OIDC Discovery document.
#[derive(Debug, Serialize)]
pub struct DiscoveryDocument {
    pub issuer: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub response_types_supported: Vec<String>,
    pub subject_types_supported: Vec<String>,
    pub id_token_signing_alg_values_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

/// JWT claims for access tokens.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct AccessTokenClaims {
    /// Subject (client_id).
    pub sub: String,
    /// Issuer.
    pub iss: String,
    /// Audience.
    pub aud: Vec<String>,
    /// Issued at timestamp.
    pub iat: i64,
    /// Expiration timestamp.
    pub exp: i64,
    /// Scopes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// JSON Web Key for JWKS endpoint.
#[derive(Debug, Serialize)]
pub struct JsonWebKey {
    pub kty: String,
    #[serde(rename = "use")]
    pub use_: String,
    pub kid: String,
    pub alg: String,
    pub n: String,
    pub e: String,
}

/// JSON Web Key Set.
#[derive(Debug, Serialize)]
pub struct JsonWebKeySet {
    pub keys: Vec<JsonWebKey>,
}
