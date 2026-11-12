use crate::{
    config::AgentAuthConfig,
    error::AgentAuthError,
    storage,
    types::{
        AuthorizeResponse, Authorized, ChallengeResponse, Delegation, StoredToken, TokenRequest,
        TokenResponse,
    },
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

/// What a failed response becomes.
///
/// Identity's 403 bodies are part of its contract (authnz-rs
/// `docs/agent-credentials-contract.md`, "Refusal bodies"): each names the
/// state that stops issuance, which is what the owner has to act on, so the
/// text is kept as sent. Any other status keeps the step it failed at.
async fn failure(
    response: reqwest::Response,
    step: fn(String) -> AgentAuthError,
) -> AgentAuthError {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::FORBIDDEN {
        return AgentAuthError::Refused(body);
    }
    step(format!("Server returned status {status}: {body}"))
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
            return Err(failure(response, AgentAuthError::ChallengeRequest).await);
        }

        let challenge: ChallengeResponse = response.json().await?;
        Ok(challenge)
    }

    /// Request a token from the server with the signed challenge.
    async fn request_token(&self, request: &TokenRequest) -> Result<TokenResponse, AgentAuthError> {
        let url = format!("{}/agents/token", self.config.base_url);

        let response = self.http_client.post(&url).json(request).send().await?;

        if !response.status().is_success() {
            return Err(failure(response, AgentAuthError::TokenRequest).await);
        }

        let token: TokenResponse = response.json().await?;
        Ok(token)
    }

    /// Authenticate with the server using the given keypair and keep the
    /// token on disk for later commands.
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
        let token = self.mint(keypair).await?;
        storage::store_at(&self.token_path()?, &token).await?;
        Ok(token)
    }

    /// Run the challenge-response flow and return the token without storing
    /// it.
    ///
    /// For a caller that needs a token minted now, whatever is cached: the
    /// market wants one no older than the request it authorizes, and a token
    /// that is spent on one request has no reason to reach the disk.
    pub async fn mint(&self, keypair: &AgentKeypair) -> Result<StoredToken, AgentAuthError> {
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

            match self.try_mint(keypair, &did).await {
                Ok(token) => {
                    tracing::info!(event = "auth_decision", action = "permit", subject = %did, "Agent authenticated");
                    return Ok(token);
                }
                // Identity has decided; asking again gets the same answer.
                Err(e @ (AgentAuthError::NotAuthorized | AgentAuthError::Refused(_))) => {
                    tracing::info!(event = "auth_decision", action = "deny", subject = %did, reason = %e, "Agent not authorized");
                    return Err(e);
                }
                Err(e) => last_error = Some(e),
            }
        }

        tracing::info!(event = "auth_decision", action = "deny", subject = %did, "Agent auth retries exhausted");
        Err(last_error.unwrap_or(AgentAuthError::TokenRequest(
            "Authentication failed after retries".into(),
        )))
    }

    async fn try_mint(
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

        Ok(StoredToken::new(
            token_response.token,
            did.to_string(),
            token_response.expires_at,
            token_response.entitlements,
        )
        .with_delegation_jwt(token_response.delegation_jwt))
    }

    /// Authorize an agent for the owner whose sign-in `owner_token` is.
    ///
    /// `owner_token` is an identity access token carrying `agents:delegate`,
    /// from a passkey sign-in no more than an hour old. It is the owner's
    /// decision, so this is never retried: a refusal or a stale sign-in needs
    /// the owner, not another attempt.
    pub async fn authorize(
        &self,
        owner_token: &str,
        delegation: &Delegation,
    ) -> Result<Authorized, AgentAuthError> {
        let url = format!("{}/agents/authorize", self.config.base_url);
        let response = self
            .http_client
            .post(&url)
            .bearer_auth(owner_token)
            .json(delegation)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(match status {
                reqwest::StatusCode::FORBIDDEN => AgentAuthError::AuthorizeRefused(body),
                reqwest::StatusCode::UNAUTHORIZED => AgentAuthError::OwnerSignInRejected(body),
                _ => AgentAuthError::AuthorizeRequest(format!(
                    "Server returned status {status}: {body}"
                )),
            });
        }

        let answer: AuthorizeResponse = response.json().await?;
        if answer.agent != delegation.agent_did || answer.state != "eligible" {
            return Err(AgentAuthError::InvalidResponse(format!(
                "identity answered for agent {} in state {}",
                answer.agent, answer.state
            )));
        }
        tracing::info!(event = "auth_decision", action = "permit", subject = %answer.agent, "Agent authorized by its owner");
        Ok(Authorized {
            appraised_until: answer.appraised_until,
        })
    }

    /// Revoke an agent's delegation as its owner.
    ///
    /// `owner_token` is the owner's `agents:delegate` sign-in token, as for
    /// [`Self::authorize`]. Identity answers the same for a delegation that
    /// was already revoked, so revoking twice is not an error. Never retried:
    /// a refusal is the owner's to act on.
    pub async fn revoke(&self, owner_token: &str, agent_did: &str) -> Result<(), AgentAuthError> {
        // The DID becomes a path segment of a request that carries the
        // owner's token, so it must be nothing but a `did:key`.
        let key = agent_did.strip_prefix("did:key:").unwrap_or_default();
        if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(AgentAuthError::RevokeRequest(format!(
                "not a did:key: {agent_did:?}"
            )));
        }

        let url = format!("{}/agents/delegations/{agent_did}", self.config.base_url);
        let response = self
            .http_client
            .delete(&url)
            .bearer_auth(owner_token)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(match status {
                reqwest::StatusCode::FORBIDDEN => AgentAuthError::RevokeRefused(body),
                reqwest::StatusCode::UNAUTHORIZED => AgentAuthError::OwnerSignInRejected(body),
                _ => AgentAuthError::RevokeRequest(format!(
                    "Server returned status {status}: {body}"
                )),
            });
        }
        tracing::info!(event = "auth_decision", action = "revoke", subject = %agent_did, "Agent delegation revoked by its owner");
        Ok(())
    }

    /// Get a valid token, either from cache or by authenticating.
    pub async fn get_token(&self, keypair: &AgentKeypair) -> Result<StoredToken, AgentAuthError> {
        // Check for cached token
        if let Some(token) = storage::load_at(&self.token_path()?).await?
            && !token.is_expired()
        {
            return Ok(token);
        }

        // Authenticate
        self.authenticate(keypair).await
    }

    /// Load a cached token without authenticating.
    pub async fn get_cached_token(&self) -> Result<Option<StoredToken>, AgentAuthError> {
        storage::load_at(&self.token_path()?).await
    }

    /// Clear the cached token.
    pub async fn clear_token(&self) -> Result<(), AgentAuthError> {
        storage::delete_at(&self.token_path()?).await
    }

    fn token_path(&self) -> Result<std::path::PathBuf, AgentAuthError> {
        match &self.config.token_path {
            Some(path) => Ok(path.clone()),
            None => storage::default_token_path(),
        }
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
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("agent_token");

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

        let config = AgentAuthConfig::new(mock_server.uri()).with_token_path(&token_path);
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
        let loaded = storage::load_at(&token_path).await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.token, stored.token);
    }

    /// Test AAUTH-001: Request authentication token — credentials rejected.
    ///
    /// When the auth server does not recognize the agent's DID, the challenge
    /// endpoint answers 404 and the client fails closed with
    /// `AgentAuthError::NotAuthorized` instead of retrying.
    #[spec("AAUTH-001")]
    #[tokio::test]
    async fn test_authenticate_rejects_unauthorized_agent() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("agent_token");

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/agents/challenge"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&mock_server)
            .await;

        let config = AgentAuthConfig::new(mock_server.uri())
            .with_max_retries(3)
            .with_token_path(&token_path);
        let client = AgentAuthClient::with_config(config).unwrap();

        let keypair = AgentKeypair::generate();
        let result = client.authenticate(&keypair).await;

        assert!(matches!(result, Err(AgentAuthError::NotAuthorized)));

        // No token may be stored for a rejected agent.
        let loaded = storage::load_at(&token_path).await.unwrap();
        assert!(loaded.is_none());
    }

    /// Test AAUTH-005: Complete challenge-response auth — invalid signature.
    ///
    /// When the server rejects the signed challenge (HTTP 401), the flow ends
    /// in an error and no token is stored.
    #[spec("AAUTH-005")]
    #[tokio::test]
    async fn test_authenticate_fails_on_rejected_signature() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("agent_token");

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

        let config = AgentAuthConfig::new(mock_server.uri())
            .with_max_retries(0)
            .with_token_path(&token_path);
        let client = AgentAuthClient::with_config(config).unwrap();

        let keypair = AgentKeypair::generate();
        let result = client.authenticate(&keypair).await;

        assert!(matches!(result, Err(AgentAuthError::TokenRequest(_))));

        let loaded = storage::load_at(&token_path).await.unwrap();
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
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("agent_token");

        let expired = StoredToken::new(
            "stale-token".to_string(),
            "did:key:z6MkStale".to_string(),
            chrono::Utc::now() - chrono::Duration::minutes(5),
            vec!["agent.capability.chat".to_string()],
        );
        storage::store_at(&token_path, &expired).await.unwrap();

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

        let config = AgentAuthConfig::new(mock_server.uri()).with_token_path(&token_path);
        let client = AgentAuthClient::with_config(config).unwrap();

        let refreshed = client.get_token(&keypair).await.unwrap();

        assert_eq!(refreshed.token, "mock-token-123");
        assert_ne!(refreshed.token, expired.token);
        assert!(!refreshed.is_expired());

        // Old token replaced: storage now holds only the new token.
        let stored = storage::load_at(&token_path).await.unwrap().unwrap();
        assert_eq!(stored.token, refreshed.token);
        assert_eq!(stored.did, refreshed.did);
        assert_eq!(stored.entitlements, refreshed.entitlements);

        let requests = mock_server.received_requests().await.unwrap();
        let paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();
        assert_eq!(paths, ["/agents/challenge", "/agents/token"]);
    }

    /// Test AAUTH-004: Refresh expired token — valid token is not refreshed.
    ///
    /// The refresh gate is expiry: while the cached token is still valid,
    /// `get_token` returns it without contacting the auth server, so a live
    /// token is never needlessly replaced.
    #[spec("AAUTH-004")]
    #[tokio::test]
    async fn test_get_token_reuses_valid_token_without_server() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("agent_token");

        let valid = StoredToken::new(
            "live-token".to_string(),
            "did:key:z6MkLive".to_string(),
            chrono::Utc::now() + chrono::Duration::hours(1),
            vec!["agent.capability.chat".to_string()],
        );
        storage::store_at(&token_path, &valid).await.unwrap();

        let mock_server = MockServer::start().await;
        let config = AgentAuthConfig::new(mock_server.uri()).with_token_path(&token_path);
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
    }

    /// Test AAUTH-006: Delete token on logout — client-facing path.
    ///
    /// `clear_token` removes the stored token so that `get_cached_token`
    /// reports nothing cached afterwards.
    #[spec("AAUTH-006")]
    #[tokio::test]
    async fn test_clear_token_removes_cached_token() {
        let dir = tempfile::tempdir().unwrap();
        let token_path = dir.path().join("agent_token");

        let token = StoredToken::new(
            "cached-token".to_string(),
            "did:key:z6MkCached".to_string(),
            chrono::Utc::now() + chrono::Duration::hours(1),
            vec!["agent.capability.chat".to_string()],
        );
        storage::store_at(&token_path, &token).await.unwrap();

        let config = AgentAuthConfig::default().with_token_path(&token_path);
        let client = AgentAuthClient::with_config(config).unwrap();
        assert!(client.get_cached_token().await.unwrap().is_some());

        client.clear_token().await.unwrap();

        assert!(client.get_cached_token().await.unwrap().is_none());
    }
}
