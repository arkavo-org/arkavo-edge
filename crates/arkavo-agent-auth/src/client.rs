use crate::{
    config::AgentAuthConfig,
    error::AgentAuthError,
    storage,
    types::{ChallengeResponse, StoredToken, TokenRequest, TokenResponse},
};
use arkavo_crypto::AgentKeypair;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use std::time::Duration;

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

/// Client for agent authentication with the authnz-rs API.
pub struct AgentAuthClient {
    config: AgentAuthConfig,
    http_client: reqwest::Client,
}

impl AgentAuthClient {
    /// Create a new authentication client with default configuration.
    pub fn new() -> Result<Self, AgentAuthError> {
        Self::with_config(AgentAuthConfig::default())
    }

    /// Create a new authentication client with custom configuration.
    pub fn with_config(config: AgentAuthConfig) -> Result<Self, AgentAuthError> {
        let http_client = reqwest::Client::builder().timeout(config.timeout).build()?;

        Ok(Self {
            config,
            http_client,
        })
    }

    /// Request a challenge from the server for the given DID.
    async fn request_challenge(&self, did: &str) -> Result<ChallengeResponse, AgentAuthError> {
        let url = format!(
            "{}/agents/challenge?did={}",
            self.config.base_url,
            percent_encode(did)
        );

        let response = self.http_client.get(&url).send().await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AgentAuthError::NotAuthorized);
        }

        if !response.status().is_success() {
            return Err(AgentAuthError::ChallengeRequest(format!(
                "Server returned status {}",
                response.status()
            )));
        }

        let challenge: ChallengeResponse = response.json().await?;
        Ok(challenge)
    }

    /// Request a token from the server with the signed challenge.
    async fn request_token(&self, request: &TokenRequest) -> Result<TokenResponse, AgentAuthError> {
        let url = format!("{}/agents/token", self.config.base_url);

        let response = self.http_client.post(&url).json(request).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AgentAuthError::TokenRequest(format!(
                "Server returned status {}: {}",
                status, body
            )));
        }

        let token: TokenResponse = response.json().await?;
        Ok(token)
    }

    /// Authenticate with the server using the given keypair.
    ///
    /// This performs the full challenge-response flow:
    /// 1. Request a challenge from the server
    /// 2. Sign the challenge with the keypair
    /// 3. Submit the signed challenge to get a token
    /// 4. Store the token locally
    pub async fn authenticate(
        &self,
        keypair: &AgentKeypair,
    ) -> Result<StoredToken, AgentAuthError> {
        let public_key = keypair.public_key();
        let did = public_key.to_did_key();

        // Retry with exponential backoff
        let mut last_error = None;
        for retry in 0..=self.config.max_retries {
            if retry > 0 {
                let delay =
                    Duration::from_millis(self.config.retry_base_delay_ms * (1 << retry.min(5)));
                tokio::time::sleep(delay).await;
            }

            match self.try_authenticate(keypair, &did).await {
                Ok(token) => {
                    tracing::info!(event = "auth_decision", action = "permit", subject = %did, "Agent authenticated");
                    return Ok(token);
                }
                Err(AgentAuthError::NotAuthorized) => {
                    tracing::info!(event = "auth_decision", action = "deny", subject = %did, "Agent not authorized");
                    return Err(AgentAuthError::NotAuthorized);
                }
                Err(e) => last_error = Some(e),
            }
        }

        tracing::info!(event = "auth_decision", action = "deny", subject = %did, "Agent auth retries exhausted");
        Err(last_error.unwrap_or(AgentAuthError::TokenRequest(
            "Authentication failed after retries".into(),
        )))
    }

    async fn try_authenticate(
        &self,
        keypair: &AgentKeypair,
        did: &str,
    ) -> Result<StoredToken, AgentAuthError> {
        // Request challenge
        let challenge_response = self.request_challenge(did).await?;

        // Decode challenge bytes from base64
        let challenge_bytes = BASE64_STANDARD
            .decode(&challenge_response.challenge)
            .map_err(|e| AgentAuthError::InvalidResponse(format!("Invalid challenge: {}", e)))?;

        // Sign the challenge
        let signature = keypair.sign(&challenge_bytes);
        let signature_b64 = BASE64_STANDARD.encode(&signature);

        // Request token
        let token_request = TokenRequest {
            did: did.to_string(),
            challenge: challenge_response.challenge,
            signature: signature_b64,
            nonce: challenge_response.nonce,
        };

        let token_response = self.request_token(&token_request).await?;

        // Create stored token
        let stored_token = StoredToken::new(
            token_response.token,
            did.to_string(),
            token_response.expires_at,
            token_response.entitlements,
        );

        // Store locally
        storage::store_token(&stored_token).await?;

        Ok(stored_token)
    }

    /// Get a valid token, either from cache or by authenticating.
    pub async fn get_token(&self, keypair: &AgentKeypair) -> Result<StoredToken, AgentAuthError> {
        // Check for cached token
        if let Some(token) = storage::load_token().await?
            && !token.is_expired()
        {
            return Ok(token);
        }

        // Authenticate
        self.authenticate(keypair).await
    }

    /// Load a cached token without authenticating.
    pub async fn get_cached_token(&self) -> Result<Option<StoredToken>, AgentAuthError> {
        storage::load_token().await
    }

    /// Clear the cached token.
    pub async fn clear_token(&self) -> Result<(), AgentAuthError> {
        storage::delete_token().await
    }
}

impl Default for AgentAuthClient {
    fn default() -> Self {
        Self::new().expect("Failed to create AgentAuthClient")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = AgentAuthClient::new();
        assert!(client.is_ok());
    }

    #[test]
    fn test_client_with_custom_config() {
        let config = AgentAuthConfig::new("https://custom.example.com")
            .with_timeout(Duration::from_secs(10))
            .with_max_retries(5);

        let client = AgentAuthClient::with_config(config);
        assert!(client.is_ok());
    }
}
