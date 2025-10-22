use crate::error::{A2aError, Result};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2Config {
    pub client_id: String,
    pub client_secret: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub issuer: String,
    pub audience: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRequest {
    pub grant_type: GrantType,
    pub code: Option<String>,
    pub refresh_token: Option<String>,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect_uri: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantType {
    AuthorizationCode,
    RefreshToken,
    ClientCredentials,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub refresh_token: Option<String>,
    pub scope: Option<String>,
    pub id_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationCode {
    pub code: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub expires_at: i64,
    pub scopes: Vec<String>,
    pub user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshToken {
    pub token: String,
    pub client_id: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessToken {
    pub token: String,
    pub client_id: String,
    pub user_id: String,
    pub scopes: Vec<String>,
    pub expires_at: i64,
}

pub struct OAuth2Provider {
    config: OAuth2Config,
    jwt_secret: String,
    authorization_codes: Arc<RwLock<HashMap<String, AuthorizationCode>>>,
    refresh_tokens: Arc<RwLock<HashMap<String, RefreshToken>>>,
    access_tokens: Arc<RwLock<HashMap<String, AccessToken>>>,
}

impl OAuth2Provider {
    pub fn new(config: OAuth2Config, jwt_secret: String) -> Self {
        Self {
            config,
            jwt_secret,
            authorization_codes: Arc::new(RwLock::new(HashMap::new())),
            refresh_tokens: Arc::new(RwLock::new(HashMap::new())),
            access_tokens: Arc::new(RwLock::new(HashMap::new())),
        }
    }

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

        info!("Generated authorization code for client: {}", client_id);
        Ok(code)
    }

    pub async fn exchange_authorization_code(
        &self,
        request: TokenRequest,
    ) -> Result<TokenResponse> {
        let code = request
            .code
            .ok_or_else(|| A2aError::Auth("Missing authorization code".to_string()))?;

        let mut codes = self.authorization_codes.write().await;
        let auth_code = codes
            .remove(&code)
            .ok_or_else(|| A2aError::Auth("Invalid authorization code".to_string()))?;

        if auth_code.expires_at < Utc::now().timestamp() {
            return Err(A2aError::Auth("Authorization code expired".to_string()));
        }

        if auth_code.client_id != request.client_id {
            return Err(A2aError::Auth("Client ID mismatch".to_string()));
        }

        if let Some(redirect_uri) = &request.redirect_uri
            && auth_code.redirect_uri != *redirect_uri
        {
            return Err(A2aError::Auth("Redirect URI mismatch".to_string()));
        }

        self.generate_tokens(auth_code.user_id, auth_code.client_id, auth_code.scopes)
            .await
    }

    pub async fn refresh_access_token(&self, request: TokenRequest) -> Result<TokenResponse> {
        let refresh_token = request
            .refresh_token
            .ok_or_else(|| A2aError::Auth("Missing refresh token".to_string()))?;

        // Clone the data we need before dropping the lock
        let (user_id, client_id, scopes) = {
            let tokens = self.refresh_tokens.read().await;
            let token_data = tokens
                .get(&refresh_token)
                .ok_or_else(|| A2aError::Auth("Invalid refresh token".to_string()))?;

            if let Some(expires_at) = token_data.expires_at
                && expires_at < Utc::now().timestamp()
            {
                return Err(A2aError::Auth("Refresh token expired".to_string()));
            }

            if token_data.client_id != request.client_id {
                return Err(A2aError::Auth("Client ID mismatch".to_string()));
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
        .map_err(|e| A2aError::Auth(format!("Failed to generate JWT: {e}")))
    }

    pub async fn revoke_token(&self, token: &str) -> Result<()> {
        let mut access_tokens = self.access_tokens.write().await;
        if access_tokens.remove(token).is_some() {
            info!("Revoked access token");
            return Ok(());
        }

        let mut refresh_tokens = self.refresh_tokens.write().await;
        if refresh_tokens.remove(token).is_some() {
            info!("Revoked refresh token");
            return Ok(());
        }

        warn!("Token not found for revocation");
        Err(A2aError::Auth("Token not found".to_string()))
    }

    pub async fn validate_access_token(&self, token: &str) -> Result<AccessToken> {
        let tokens = self.access_tokens.read().await;
        let token_data = tokens
            .get(token)
            .ok_or_else(|| A2aError::Auth("Invalid access token".to_string()))?;

        if token_data.expires_at < Utc::now().timestamp() {
            return Err(A2aError::Auth("Access token expired".to_string()));
        }

        Ok(token_data.clone())
    }

    pub fn get_authorization_url(&self, state: &str) -> String {
        format!(
            "{}?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
            self.config.authorization_endpoint,
            self.config.client_id,
            urlencoding::encode(&self.config.redirect_uri),
            urlencoding::encode(&self.config.scopes.join(" ")),
            state
        )
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;

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
