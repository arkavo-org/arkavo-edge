//! The token exchange against identity's wire format.
//!
//! The bodies here are written as authnz-rs sends them (`src/agent.rs`,
//! `ChallengeResponse` and `TokenResponse`), not built from this crate's own
//! types: a test that serializes the client's struct agrees with the client
//! whatever the server does.

// `#[tokio::test]` expands to `Runtime::block_on`, which `.clippy.toml`
// disallows in the product, where nesting runtimes can panic.
#![allow(clippy::disallowed_methods)]

use arkavo_agent_auth::{AgentAuthClient, AgentAuthConfig, AgentAuthError};
use arkavo_crypto::AgentKeypair;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MARKET_REQUEST: &str = "https://arkavo.net/attr/market/value/request";

async fn identity_with_challenge() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/agents/challenge"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "challenge": STANDARD.encode([7u8; 32]),
            "nonce": "nonce-1",
        })))
        .mount(&server)
        .await;
    server
}

fn client(server: &MockServer) -> AgentAuthClient {
    AgentAuthClient::with_config(AgentAuthConfig::new(server.uri()).with_max_retries(0)).unwrap()
}

/// Regression: identity answers `expires_at` as Unix seconds, and the client
/// read it as an RFC 3339 string, so no token identity issued could be parsed.
#[tokio::test]
async fn mint_reads_the_token_response_identity_sends() {
    let server = identity_with_challenge().await;
    let expires_at = chrono::Utc::now().timestamp() + 900;
    Mock::given(method("POST"))
        .and(path("/agents/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "token": "2D3ShEOhASag",
            "expires_at": expires_at,
            "entitlements": [MARKET_REQUEST],
        })))
        .mount(&server)
        .await;

    let keypair = AgentKeypair::generate();
    let token = client(&server).mint(&keypair).await.unwrap();

    assert_eq!(token.token, "2D3ShEOhASag");
    assert_eq!(token.did, keypair.public_key().to_did_key());
    assert_eq!(token.expires_at.timestamp(), expires_at);
    assert_eq!(token.entitlements, [MARKET_REQUEST]);
    assert!(!token.is_expired());
}

/// Regression: the default host was `100.arkavo.net`, which serves no agent
/// endpoints; they live on identity.
#[test]
fn the_default_host_is_identity() {
    assert_eq!(
        AgentAuthConfig::default().base_url,
        "https://identity.arkavo.net"
    );
}

/// Identity's 403 bodies say why it will not issue a token (quarantined,
/// appraisal expired, delegation revoked). They are final, so the client
/// reports the text and does not ask again.
#[tokio::test]
async fn a_refusal_is_reported_in_identity_words_and_not_retried() {
    let server = MockServer::start().await;
    let refusal = "Forbidden: agent appraisal expired; it needs a fresh appraisal";
    Mock::given(method("GET"))
        .and(path("/agents/challenge"))
        .respond_with(ResponseTemplate::new(403).set_body_string(refusal))
        .mount(&server)
        .await;
    let client =
        AgentAuthClient::with_config(AgentAuthConfig::new(server.uri()).with_max_retries(3))
            .unwrap();

    let err = client.mint(&AgentKeypair::generate()).await.unwrap_err();

    assert!(matches!(&err, AgentAuthError::Refused(text) if text == refusal));
    assert!(err.to_string().contains(refusal), "{err}");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// A refusal at the token step is as final as one at the challenge.
#[tokio::test]
async fn a_refusal_at_the_token_step_is_reported_too() {
    let server = identity_with_challenge().await;
    Mock::given(method("POST"))
        .and(path("/agents/token"))
        .respond_with(ResponseTemplate::new(403).set_body_string("Workload quarantined"))
        .mount(&server)
        .await;

    let err = client(&server)
        .mint(&AgentKeypair::generate())
        .await
        .unwrap_err();

    assert!(matches!(&err, AgentAuthError::Refused(text) if text == "Workload quarantined"));
}
