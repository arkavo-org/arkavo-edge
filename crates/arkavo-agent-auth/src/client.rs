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
        )
        .with_delegation_jwt(token_response.delegation_jwt);

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
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_crypto::{AgentKeypair, AgentPublicKey};
    use arkavo_test_macros::spec;
    use wiremock::Respond;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    use crate::test_helpers::TEST_LOCK;

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

    /// Wiremock responder that validates the Ed25519 signature on the token
    /// request before returning a token response.
    struct ValidatingTokenResponder {
        public_key: AgentPublicKey,
        challenge: Vec<u8>,
    }

    impl ValidatingTokenResponder {
        fn new(public_key: AgentPublicKey, challenge: Vec<u8>) -> Self {
            Self {
                public_key,
                challenge,
            }
        }
    }

    impl Respond for ValidatingTokenResponder {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            let body: TokenRequest = match serde_json::from_slice(&request.body) {
                Ok(b) => b,
                Err(_) => return ResponseTemplate::new(400).set_body_string("invalid json"),
            };

            if body.challenge != BASE64_STANDARD.encode(&self.challenge) {
                return ResponseTemplate::new(400).set_body_string("challenge mismatch");
            }

            let signature = match BASE64_STANDARD.decode(&body.signature) {
                Ok(s) => s,
                Err(_) => return ResponseTemplate::new(400).set_body_string("bad signature"),
            };

            if self.public_key.verify(&self.challenge, &signature).is_err() {
                return ResponseTemplate::new(401).set_body_string("invalid signature");
            }

            ResponseTemplate::new(200).set_body_json(TokenResponse {
                token: "mock-token-123".to_string(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                entitlements: vec!["agent.capability.chat".to_string()],
                delegation_jwt: Some("mock-delegation-jwt".to_string()),
            })
        }
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
            vec!["agent.capability.chat".to_string()]
        );
        assert_eq!(
            stored.delegation_jwt,
            Some("mock-delegation-jwt".to_string())
        );
        assert!(!stored.is_expired());

        // Verify the token was persisted to storage.
        let loaded = storage::load_token().await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.token, stored.token);

        let _ = storage::delete_token().await;
    }

    /// Test AAUTH-001: Request authentication token — credentials rejected.
    ///
    /// When the auth server does not recognize the agent's DID, the challenge
    /// endpoint answers 404 and the client fails closed with
    /// `AgentAuthError::NotAuthorized` instead of retrying.
    #[spec("AAUTH-001")]
    #[tokio::test]
    async fn test_authenticate_rejects_unauthorized_agent() {
        let _guard = TEST_LOCK.lock().await;
        let _ = storage::delete_token().await;

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/agents/challenge"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let config = AgentAuthConfig::new(mock_server.uri()).with_max_retries(3);
        let client = AgentAuthClient::with_config(config).unwrap();

        let keypair = AgentKeypair::generate();
        let result = client.authenticate(&keypair).await;

        assert!(matches!(result, Err(AgentAuthError::NotAuthorized)));

        // No token may be stored for a rejected agent.
        let loaded = storage::load_token().await.unwrap();
        assert!(loaded.is_none());
    }

    /// Test AAUTH-005: Complete challenge-response auth — invalid signature.
    ///
    /// When the server rejects the signed challenge (HTTP 401), the flow ends
    /// in an error and no token is stored.
    #[spec("AAUTH-005")]
    #[tokio::test]
    async fn test_authenticate_fails_on_rejected_signature() {
        let _guard = TEST_LOCK.lock().await;
        let _ = storage::delete_token().await;

        let mock_server = MockServer::start().await;

        let challenge = b"challenge-data-for-test".to_vec();
        Mock::given(method("GET"))
            .and(path("/agents/challenge"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ChallengeResponse {
                challenge: BASE64_STANDARD.encode(&challenge),
                nonce: "nonce-123".to_string(),
            }))
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/agents/token"))
            .respond_with(ResponseTemplate::new(401).set_body_string("invalid signature"))
            .mount(&mock_server)
            .await;

        let config = AgentAuthConfig::new(mock_server.uri()).with_max_retries(0);
        let client = AgentAuthClient::with_config(config).unwrap();

        let keypair = AgentKeypair::generate();
        let result = client.authenticate(&keypair).await;

        assert!(matches!(result, Err(AgentAuthError::TokenRequest(_))));

        let loaded = storage::load_token().await.unwrap();
        assert!(loaded.is_none());
    }

    /// Test AAUTH-004: Refresh expired token.
    ///
    /// The spec names a standalone `refresh_token()` driven by a refresh token,
    /// which does not exist in the current implementation: `TokenResponse`
    /// carries no refresh token. The implemented refresh path is `get_token` —
    /// an expired cached token triggers re-authentication and the stored token
    /// is replaced with the newly issued one.
    #[spec("AAUTH-004")]
    #[tokio::test]
    async fn test_get_token_replaces_expired_token() {
        let _guard = TEST_LOCK.lock().await;
        let _ = storage::delete_token().await;

        let expired = StoredToken::new(
            "stale-token".to_string(),
            "did:key:z6MkStale".to_string(),
            chrono::Utc::now() - chrono::Duration::minutes(5),
            vec!["agent.capability.chat".to_string()],
        );
        storage::store_token(&expired).await.unwrap();

        let mock_server = MockServer::start().await;

        let keypair = AgentKeypair::generate();
        let challenge = b"refresh-challenge".to_vec();
        Mock::given(method("GET"))
            .and(path("/agents/challenge"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ChallengeResponse {
                challenge: BASE64_STANDARD.encode(&challenge),
                nonce: "nonce-refresh".to_string(),
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

        let refreshed = client.get_token(&keypair).await.unwrap();

        assert_eq!(refreshed.token, "mock-token-123");
        assert_ne!(refreshed.token, expired.token);
        assert!(!refreshed.is_expired());

        // Old token replaced: storage now holds only the new token.
        let stored = storage::load_token().await.unwrap().unwrap();
        assert_eq!(stored.token, refreshed.token);
        assert_eq!(stored.did, refreshed.did);
        assert_eq!(stored.entitlements, refreshed.entitlements);

        let requests = mock_server.received_requests().await.unwrap();
        let paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();
        assert_eq!(paths, ["/agents/challenge", "/agents/token"]);

        let _ = storage::delete_token().await;
    }

    /// Test AAUTH-004: Refresh expired token — valid token is not refreshed.
    ///
    /// The refresh gate is expiry: while the cached token is still valid,
    /// `get_token` returns it without contacting the auth server, so a live
    /// token is never needlessly replaced.
    #[spec("AAUTH-004")]
    #[tokio::test]
    async fn test_get_token_reuses_valid_token_without_server() {
        let _guard = TEST_LOCK.lock().await;
        let _ = storage::delete_token().await;

        let valid = StoredToken::new(
            "live-token".to_string(),
            "did:key:z6MkLive".to_string(),
            chrono::Utc::now() + chrono::Duration::hours(1),
            vec!["agent.capability.chat".to_string()],
        );
        storage::store_token(&valid).await.unwrap();

        let mock_server = MockServer::start().await;
        let config = AgentAuthConfig::new(mock_server.uri());
        let client = AgentAuthClient::with_config(config).unwrap();

        let keypair = AgentKeypair::generate();
        let token = client.get_token(&keypair).await.unwrap();

        assert_eq!(token.token, "live-token");
        assert_eq!(token.did, valid.did);

        let requests = mock_server.received_requests().await.unwrap();
        assert!(
            requests.is_empty(),
            "valid cached token must not trigger a token request"
        );

        let _ = storage::delete_token().await;
    }

    /// Test AAUTH-006: Delete token on logout — client-facing path.
    ///
    /// `clear_token` removes the stored token so that `get_cached_token`
    /// reports nothing cached afterwards.
    #[spec("AAUTH-006")]
    #[tokio::test]
    async fn test_clear_token_removes_cached_token() {
        let _guard = TEST_LOCK.lock().await;
        let _ = storage::delete_token().await;

        let token = StoredToken::new(
            "cached-token".to_string(),
            "did:key:z6MkCached".to_string(),
            chrono::Utc::now() + chrono::Duration::hours(1),
            vec!["agent.capability.chat".to_string()],
        );
        storage::store_token(&token).await.unwrap();

        let client = AgentAuthClient::new().unwrap();
        assert!(client.get_cached_token().await.unwrap().is_some());

        client.clear_token().await.unwrap();

        assert!(client.get_cached_token().await.unwrap().is_none());

        let _ = storage::delete_token().await;
    }
}
