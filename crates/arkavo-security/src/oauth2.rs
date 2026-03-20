//! OAuth2 provider implementation

#![allow(clippy::significant_drop_tightening)]

use crate::error::{Result, SecurityError};
use chrono::{Duration, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

fn percent_encode(input: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

/// OAuth2 client configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Config {
    /// Client ID
    pub client_id: String,
    /// Client secret
    pub client_secret: String,
    /// Authorization endpoint URL
    pub authorization_endpoint: String,
    /// Token endpoint URL
    pub token_endpoint: String,
    /// Redirect URI
    pub redirect_uri: String,
    /// Requested scopes
    pub scopes: Vec<String>,
    /// Token issuer
    pub issuer: String,
    /// Token audience
    pub audience: Option<String>,
}

/// Token request parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRequest {
    /// Grant type
    pub grant_type: GrantType,
    /// Authorization code
    pub code: Option<String>,
    /// Refresh token
    pub refresh_token: Option<String>,
    /// Client ID
    pub client_id: String,
    /// Client secret
    pub client_secret: Option<String>,
    /// Redirect URI
    pub redirect_uri: Option<String>,
    /// Requested scope
    pub scope: Option<String>,
}

/// OAuth2 grant types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantType {
    /// Authorization code grant
    AuthorizationCode,
    /// Refresh token grant
    RefreshToken,
    /// Client credentials grant
    ClientCredentials,
}

/// Token response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    /// Access token
    pub access_token: String,
    /// Token type (Bearer)
    pub token_type: String,
    /// Expiration time in seconds
    pub expires_in: i64,
    /// Refresh token
    pub refresh_token: Option<String>,
    /// Granted scope
    pub scope: Option<String>,
    /// ID token (OIDC)
    pub id_token: Option<String>,
}

/// Authorization code data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationCode {
    /// The code
    pub code: String,
    /// Client ID
    pub client_id: String,
    /// Redirect URI
    pub redirect_uri: String,
    /// Expiration timestamp
    pub expires_at: i64,
    /// Granted scopes
    pub scopes: Vec<String>,
    /// User ID
    pub user_id: String,
}

/// Refresh token data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshToken {
    /// The token
    pub token: String,
    /// Client ID
    pub client_id: String,
    /// User ID
    pub user_id: String,
    /// Granted scopes
    pub scopes: Vec<String>,
    /// Expiration timestamp
    pub expires_at: Option<i64>,
}

/// Access token data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessToken {
    /// The token
    pub token: String,
    /// Client ID
    pub client_id: String,
    /// User ID
    pub user_id: String,
    /// Granted scopes
    pub scopes: Vec<String>,
    /// Expiration timestamp
    pub expires_at: i64,
}

/// OAuth2 provider
pub struct OAuth2Provider {
    config: OAuth2Config,
    jwt_secret: String,
    authorization_codes: Arc<RwLock<HashMap<String, AuthorizationCode>>>,
    refresh_tokens: Arc<RwLock<HashMap<String, RefreshToken>>>,
    access_tokens: Arc<RwLock<HashMap<String, AccessToken>>>,
}

impl OAuth2Provider {
    /// Create a new OAuth2 provider
    pub fn new(config: OAuth2Config, jwt_secret: String) -> Self {
        Self {
            config,
            jwt_secret,
            authorization_codes: Arc::new(RwLock::new(HashMap::new())),
            refresh_tokens: Arc::new(RwLock::new(HashMap::new())),
            access_tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generate an authorization code
    pub async fn generate_authorization_code(
        &self,
        client_id: String,
        redirect_uri: String,
        scopes: Vec<String>,
        user_id: String,
    ) -> Result<String> {
        let code = Uuid::new_v4().to_string();
        let expires_at = Utc::now().timestamp() + 600; // 10 minutes

        let auth_code = AuthorizationCode {
            code: code.clone(),
            client_id: client_id.clone(),
            redirect_uri,
            expires_at,
            scopes,
            user_id,
        };

        let mut codes = self.authorization_codes.write().await;
        codes.insert(code.clone(), auth_code);

        info!(event = "auth_decision", action = "permit", subject = %client_id, resource = "oauth2_authz", "Generated authorization code");
        Ok(code)
    }

    /// Exchange authorization code for tokens
    pub async fn exchange_authorization_code(
        &self,
        request: TokenRequest,
    ) -> Result<TokenResponse> {
        let code = request
            .code
            .ok_or_else(|| SecurityError::OAuth2("Missing authorization code".to_string()))?;

        let mut codes = self.authorization_codes.write().await;
        let auth_code = codes
            .remove(&code)
            .ok_or_else(|| SecurityError::OAuth2("Invalid authorization code".to_string()))?;

        if auth_code.expires_at < Utc::now().timestamp() {
            return Err(SecurityError::OAuth2(
                "Authorization code expired".to_string(),
            ));
        }

        if auth_code.client_id != request.client_id {
            return Err(SecurityError::OAuth2("Client ID mismatch".to_string()));
        }

        if let Some(redirect_uri) = &request.redirect_uri
            && auth_code.redirect_uri != *redirect_uri
        {
            return Err(SecurityError::OAuth2("Redirect URI mismatch".to_string()));
        }

        self.generate_tokens(auth_code.user_id, auth_code.client_id, auth_code.scopes)
            .await
    }

    /// Refresh access token
    pub async fn refresh_access_token(&self, request: TokenRequest) -> Result<TokenResponse> {
        let refresh_token = request
            .refresh_token
            .ok_or_else(|| SecurityError::OAuth2("Missing refresh token".to_string()))?;

        // Clone the data we need before dropping the lock
        let (user_id, client_id, scopes) = {
            let tokens = self.refresh_tokens.read().await;
            let token_data = tokens
                .get(&refresh_token)
                .ok_or_else(|| SecurityError::OAuth2("Invalid refresh token".to_string()))?;

            if let Some(expires_at) = token_data.expires_at
                && expires_at < Utc::now().timestamp()
            {
                return Err(SecurityError::OAuth2("Refresh token expired".to_string()));
            }

            if token_data.client_id != request.client_id {
                return Err(SecurityError::OAuth2("Client ID mismatch".to_string()));
            }

            (
                token_data.user_id.clone(),
                token_data.client_id.clone(),
                token_data.scopes.clone(),
            )
        }; // Lock is dropped here

        self.generate_tokens(user_id, client_id, scopes).await
    }

    async fn generate_tokens(
        &self,
        user_id: String,
        client_id: String,
        scopes: Vec<String>,
    ) -> Result<TokenResponse> {
        let access_token = self.generate_jwt_token(&user_id, &client_id, &scopes, 3600)?;
        let refresh_token = Uuid::new_v4().to_string();

        let refresh_token_data = RefreshToken {
            token: refresh_token.clone(),
            client_id: client_id.clone(),
            user_id: user_id.clone(),
            scopes: scopes.clone(),
            expires_at: Some(Utc::now().timestamp() + 2_592_000), // 30 days
        };

        let access_token_data = AccessToken {
            token: access_token.clone(),
            client_id,
            user_id,
            scopes: scopes.clone(),
            expires_at: Utc::now().timestamp() + 3600,
        };

        let mut refresh_tokens = self.refresh_tokens.write().await;
        refresh_tokens.insert(refresh_token.clone(), refresh_token_data);

        let mut access_tokens = self.access_tokens.write().await;
        access_tokens.insert(access_token.clone(), access_token_data);

        Ok(TokenResponse {
            access_token,
            token_type: "Bearer".to_string(),
            expires_in: 3600,
            refresh_token: Some(refresh_token),
            scope: Some(scopes.join(" ")),
            id_token: None,
        })
    }

    fn generate_jwt_token(
        &self,
        user_id: &str,
        client_id: &str,
        scopes: &[String],
        expires_in: i64,
    ) -> Result<String> {
        let exp = Utc::now() + Duration::seconds(expires_in);

        let claims = serde_json::json!({
            "sub": user_id,
            "client_id": client_id,
            "scopes": scopes,
            "iss": self.config.issuer,
            "aud": self.config.audience,
            "exp": exp.timestamp(),
            "iat": Utc::now().timestamp(),
            "jti": Uuid::new_v4().to_string(),
        });

        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_ref()),
        )
        .map_err(|e| SecurityError::Jwt(format!("Failed to generate JWT: {e}")))
    }

    /// Revoke a token
    pub async fn revoke_token(&self, token: &str) -> Result<()> {
        let mut access_tokens = self.access_tokens.write().await;
        if access_tokens.remove(token).is_some() {
            info!(
                event = "auth_decision",
                action = "deny",
                resource = "oauth2_access_token",
                "Revoked access token"
            );
            return Ok(());
        }

        let mut refresh_tokens = self.refresh_tokens.write().await;
        if refresh_tokens.remove(token).is_some() {
            info!(
                event = "auth_decision",
                action = "deny",
                resource = "oauth2_refresh_token",
                "Revoked refresh token"
            );
            return Ok(());
        }

        warn!("Token not found for revocation");
        Err(SecurityError::OAuth2("Token not found".to_string()))
    }

    /// Validate an access token
    pub async fn validate_access_token(&self, token: &str) -> Result<AccessToken> {
        let tokens = self.access_tokens.read().await;
        let token_data = tokens
            .get(token)
            .ok_or_else(|| SecurityError::OAuth2("Invalid access token".to_string()))?;

        if token_data.expires_at < Utc::now().timestamp() {
            return Err(SecurityError::OAuth2("Access token expired".to_string()));
        }

        Ok(token_data.clone())
    }

    /// Get the authorization URL
    pub fn get_authorization_url(&self, state: &str) -> String {
        format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
            self.config.authorization_endpoint,
            self.config.client_id,
            percent_encode(&self.config.redirect_uri),
            percent_encode(&self.config.scopes.join(" ")),
            state
        )
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn create_test_config() -> OAuth2Config {
        OAuth2Config {
            client_id: "test_client".to_string(),
            client_secret: "test_secret".to_string(),
            authorization_endpoint: "https://auth.example.com/authorize".to_string(),
            token_endpoint: "https://auth.example.com/token".to_string(),
            redirect_uri: "https://app.example.com/callback".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            issuer: "https://auth.example.com".to_string(),
            audience: Some("https://api.example.com".to_string()),
        }
    }

    #[spec("SEC-002")]
    #[tokio::test]
    async fn test_authorization_code_flow() {
        let config = create_test_config();
        let provider = OAuth2Provider::new(config, "test_jwt_secret".to_string());

        let code = provider
            .generate_authorization_code(
                "test_client".to_string(),
                "https://app.example.com/callback".to_string(),
                vec!["read".to_string()],
                "user123".to_string(),
            )
            .await
            .unwrap();

        let token_request = TokenRequest {
            grant_type: GrantType::AuthorizationCode,
            code: Some(code),
            refresh_token: None,
            client_id: "test_client".to_string(),
            client_secret: Some("test_secret".to_string()),
            redirect_uri: Some("https://app.example.com/callback".to_string()),
            scope: None,
        };

        let response = provider
            .exchange_authorization_code(token_request)
            .await
            .unwrap();

        assert_eq!(response.token_type, "Bearer");
        assert_eq!(response.expires_in, 3600);
        assert!(response.refresh_token.is_some());
    }

    #[spec("SEC-002")]
    #[tokio::test]
    async fn test_refresh_token_flow() {
        let config = create_test_config();
        let provider = OAuth2Provider::new(config, "test_jwt_secret".to_string());

        let code = provider
            .generate_authorization_code(
                "test_client".to_string(),
                "https://app.example.com/callback".to_string(),
                vec!["read".to_string()],
                "user123".to_string(),
            )
            .await
            .unwrap();

        let token_request = TokenRequest {
            grant_type: GrantType::AuthorizationCode,
            code: Some(code),
            refresh_token: None,
            client_id: "test_client".to_string(),
            client_secret: Some("test_secret".to_string()),
            redirect_uri: Some("https://app.example.com/callback".to_string()),
            scope: None,
        };

        let initial_response = provider
            .exchange_authorization_code(token_request)
            .await
            .unwrap();

        let refresh_request = TokenRequest {
            grant_type: GrantType::RefreshToken,
            code: None,
            refresh_token: initial_response.refresh_token,
            client_id: "test_client".to_string(),
            client_secret: Some("test_secret".to_string()),
            redirect_uri: None,
            scope: None,
        };

        let refreshed_response = provider
            .refresh_access_token(refresh_request)
            .await
            .unwrap();

        assert_eq!(refreshed_response.token_type, "Bearer");
        assert!(refreshed_response.refresh_token.is_some());
    }

    #[spec("SEC-002")]
    #[tokio::test]
    async fn test_token_validation() {
        let config = create_test_config();
        let provider = OAuth2Provider::new(config, "test_jwt_secret".to_string());

        let tokens = provider
            .generate_tokens(
                "user123".to_string(),
                "test_client".to_string(),
                vec!["read".to_string()],
            )
            .await
            .unwrap();

        let validation_result = provider
            .validate_access_token(&tokens.access_token)
            .await
            .unwrap();

        assert_eq!(validation_result.user_id, "user123");
        assert_eq!(validation_result.client_id, "test_client");
    }
}
