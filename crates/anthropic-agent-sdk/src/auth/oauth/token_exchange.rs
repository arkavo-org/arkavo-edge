//! OAuth token exchange and refresh operations

use super::{AuthResult, ErrorResponse, OAuthClient, OAuthError, TokenResponse};
use crate::auth::token::TokenInfo;

impl OAuthClient {
    /// Exchange authorization code for access token
    pub(super) async fn exchange_code(
        &self,
        code: &str,
        state: Option<&str>,
        code_verifier: &str,
    ) -> AuthResult<TokenInfo> {
        // Build JSON body - Anthropic requires JSON, not form-urlencoded
        let mut body = serde_json::json!({
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": self.config.redirect_uri,
            "client_id": self.config.client_id,
            "code_verifier": code_verifier
        });

        // Include state if provided (from code#state format)
        if let Some(state_val) = state {
            body["state"] = serde_json::json!(state_val);
        }

        let response = self
            .http_client
            .post(&self.config.token_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await?;

        // Check HTTP status first
        if !status.is_success() {
            if let Ok(error) = serde_json::from_str::<ErrorResponse>(&response_text) {
                let msg = error.error_description.unwrap_or(error.error);
                return Err(OAuthError::TokenExchange(msg));
            }
            return Err(OAuthError::TokenExchange(format!(
                "HTTP {status}: {response_text}"
            )));
        }

        // Parse token response
        let token_response: TokenResponse = serde_json::from_str(&response_text).map_err(|e| {
            OAuthError::InvalidResponse(format!(
                "Failed to parse token response: {e} - Response: {response_text}"
            ))
        })?;

        Ok(TokenInfo::new(
            token_response.access_token,
            token_response.refresh_token,
            token_response.expires_in,
            token_response.scope,
        ))
    }

    /// Refresh an expired token
    pub(super) async fn refresh_token(&self, refresh_token: &str) -> AuthResult<TokenInfo> {
        // Anthropic requires JSON, not form-urlencoded
        let body = serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": self.config.client_id
        });

        let response = self
            .http_client
            .post(&self.config.token_url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await?;

        // Check HTTP status first
        if !status.is_success() {
            if let Ok(error) = serde_json::from_str::<ErrorResponse>(&response_text) {
                let msg = error.error_description.unwrap_or(error.error);
                return Err(OAuthError::TokenExchange(msg));
            }
            return Err(OAuthError::TokenExchange(format!(
                "HTTP {status}: {response_text}"
            )));
        }

        let token_response: TokenResponse = serde_json::from_str(&response_text).map_err(|e| {
            OAuthError::InvalidResponse(format!("Failed to parse refresh response: {e}"))
        })?;

        let token = TokenInfo::new(
            token_response.access_token,
            token_response
                .refresh_token
                .or_else(|| Some(refresh_token.to_string())),
            token_response.expires_in,
            token_response.scope,
        );

        self.storage.save(&token)?;

        Ok(token)
    }
}
