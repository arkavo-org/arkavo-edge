use crate::{
    config::AgentAuthConfig,
    error::AgentAuthError,
    storage,
    types::{ChallengeResponse, StoredToken, TokenRequest, TokenResponse},
};
use arkavo_crypto::AgentKeypair;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::Utc;
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
        let status = response.status();

        if status == reqwest::StatusCode::FORBIDDEN {
            let body = response.text().await.unwrap_or_default();
            return Err(AgentAuthError::Forbidden(body));
        }

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(AgentAuthError::NotAuthorized);
        }

        if !status.is_success() {
            return Err(AgentAuthError::ChallengeRequest(format!(
                "Server returned status {}",
                status
            )));
        }

        let challenge: ChallengeResponse = response.json().await?;
        Ok(challenge)
    }

    /// Request a token from the server with the signed challenge.
    async fn request_token(&self, request: &TokenRequest) -> Result<TokenResponse, AgentAuthError> {
        let url = format!("{}/agents/token", self.config.base_url);

        let response = self.http_client.post(&url).json(request).send().await?;
        let status = response.status();

        if status == reqwest::StatusCode::FORBIDDEN {
            let body = response.text().await.unwrap_or_default();
            return Err(AgentAuthError::Forbidden(body));
        }

        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(AgentAuthError::NotAuthorized);
        }

        if !status.is_success() {
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
                Err(AgentAuthError::Forbidden(reason)) => {
                    // Revoked/expired delegation: burning retries against a
                    // server that will keep saying no just delays the human
                    // finding out the agent needs to be re-trusted.
                    tracing::info!(event = "auth_decision", action = "deny", subject = %did, reason = %reason, "Agent delegation revoked");
                    storage::delete_token().await.ok();
                    return Err(AgentAuthError::Forbidden(reason));
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
    ///
    /// A cached token is reused only while it is neither expired nor within
    /// the last third of its lifetime; "refresh" always means re-running the
    /// challenge-response flow, since there are no refresh tokens.
    pub async fn get_token(&self, keypair: &AgentKeypair) -> Result<StoredToken, AgentAuthError> {
        if let Some(token) = storage::load_token().await?
            && !token.is_expired()
            && !token.needs_refresh(Utc::now())
        {
            return Ok(token);
        }

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
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use arkavo_test_macros::spec;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::test_helpers::{TEST_LOCK, ValidatingTokenResponder};

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

    /// Test AAUTH-001: Request authentication token.
    /// Test AAUTH-005: Complete challenge-response auth.
    ///
    /// The spec names a standalone `respond_to_challenge()` function, which does
    /// not exist in the current implementation. The equivalent behavior is
    /// exercised through `AgentAuthClient::authenticate`, which performs the full
    /// challenge-response flow, submits the signed request to the token endpoint,
    /// and stores the resulting token.
    #[spec("AAUTH-001", "AAUTH-005")]
    #[tokio::test]
    async fn test_authenticate_challenge_response_flow() {
        let _guard = TEST_LOCK.lock().await;
        let _ = storage::delete_token().await;

        let mock_server = MockServer::start().await;

        let keypair = AgentKeypair::generate();
        let challenge = b"challenge-data-for-test".to_vec();
        let challenge_b64 = BASE64_STANDARD.encode(&challenge);
        let did = keypair.public_key().to_did_key();

        Mock::given(method("GET"))
            .and(path("/agents/challenge"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ChallengeResponse {
                challenge: challenge_b64,
                nonce: "nonce-123".to_string(),
            }))
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/agents/token"))
            .respond_with(ValidatingTokenResponder::new(
                keypair.public_key(),
                challenge,
            ))
            .mount(&mock_server)
            .await;

        let config = AgentAuthConfig::new(mock_server.uri());
        let client = AgentAuthClient::with_config(config).unwrap();

        let stored = client.authenticate(&keypair).await.unwrap();

        assert_eq!(stored.token, "mock-token-123");
        assert_eq!(stored.did, did);
        assert_eq!(
            stored.entitlements,
            vec!["https://arkavo.ai/attr/tdf/value/decrypt".to_string()]
        );
        assert!(!stored.is_expired());

        // Verify the token was persisted to storage.
        let loaded = storage::load_token().await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.token, stored.token);

        let _ = storage::delete_token().await;
    }

    /// Regression test for C1/C4: a 403 from `/agents/challenge` (revoked
    /// delegation) must map to `AgentAuthError::Forbidden`, must not be
    /// retried, and must clear any stored token immediately.
    #[tokio::test]
    #[spec("AAUTH-007")]
    async fn forbidden_challenge_maps_to_forbidden_and_clears_token() {
        let _g = TEST_LOCK.lock().await;
        storage::delete_token().await.unwrap();

        let keypair = AgentKeypair::generate();
        let stale = StoredToken::new(
            "stale-token".into(),
            keypair.public_key().to_did_key(),
            Utc::now() + chrono::Duration::minutes(15),
            vec![],
        );
        storage::store_token(&stale).await.unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/agents/challenge"))
            .respond_with(ResponseTemplate::new(403).set_body_string("Delegation revoked"))
            .mount(&server)
            .await;

        let client = AgentAuthClient::with_config(AgentAuthConfig::new(server.uri())).unwrap();
        let err = client.authenticate(&keypair).await.unwrap_err();
        assert!(matches!(err, AgentAuthError::Forbidden(_)));
        assert!(storage::load_token().await.unwrap().is_none());

        storage::delete_token().await.unwrap();
    }

    #[tokio::test]
    async fn not_found_challenge_is_not_authorized_yet() {
        let _g = TEST_LOCK.lock().await;
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/agents/challenge"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = AgentAuthClient::with_config(AgentAuthConfig::new(server.uri())).unwrap();
        assert!(matches!(
            client
                .authenticate(&AgentKeypair::generate())
                .await
                .unwrap_err(),
            AgentAuthError::NotAuthorized
        ));
    }
}
