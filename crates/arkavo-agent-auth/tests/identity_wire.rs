//! The token exchange against identity's wire format.
//!
//! The bodies here are written as authnz-rs sends them (`src/agent.rs`,
//! `ChallengeResponse` and `TokenResponse`), not built from this crate's own
//! types: a test that serializes the client's struct agrees with the client
//! whatever the server does.

// `#[tokio::test]` expands to `Runtime::block_on`, which `.clippy.toml`
// disallows in the product, where nesting runtimes can panic.
#![allow(clippy::disallowed_methods)]

use arkavo_agent_auth::{AgentAuthClient, AgentAuthConfig, AgentAuthError, Delegation};
use arkavo_crypto::AgentKeypair;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;
use wiremock::matchers::{body_json, header, method, path};
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

const AGENT: &str = "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK";

fn market_delegation() -> Delegation {
    Delegation {
        agent_did: AGENT.to_string(),
        name: "Arkavo Edge".to_string(),
        entitlements: vec![MARKET_REQUEST.to_string()],
        short_lived: true,
    }
}

/// The owner's authorize, as contract v2 words it: the owner's sign-in token
/// as a Bearer, and an answer that says until when the agent may mint.
#[tokio::test]
async fn authorize_sends_the_delegation_and_reads_identitys_answer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agents/authorize"))
        .and(header("authorization", "Bearer owner-token"))
        .and(body_json(json!({
            "agent_did": AGENT,
            "name": "Arkavo Edge",
            "entitlements": [MARKET_REQUEST],
            "short_lived": true,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "message": "Agent authorized successfully",
            "agent": AGENT,
            "state": "eligible",
            "state_version": 1,
            "appraised_until": 1_790_043_200,
        })))
        .mount(&server)
        .await;

    let authorized = client(&server)
        .authorize("owner-token", &market_delegation())
        .await
        .unwrap();

    assert_eq!(authorized.appraised_until.timestamp(), 1_790_043_200);
}

/// Identity's 403 says what the owner has to change (an entitlement they do
/// not hold, a quarantined agent), so its words reach the owner.
#[tokio::test]
async fn a_refused_authorize_is_reported_in_identity_words() {
    let server = MockServer::start().await;
    let refusal =
        "Entitlement 'https://arkavo.ai/attr/tdf/value/decrypt' is not held by the delegator";
    Mock::given(method("POST"))
        .and(path("/agents/authorize"))
        .respond_with(ResponseTemplate::new(403).set_body_string(refusal))
        .mount(&server)
        .await;

    let err = client(&server)
        .authorize("owner-token", &market_delegation())
        .await
        .unwrap_err();

    assert!(matches!(&err, AgentAuthError::AuthorizeRefused(text) if text == refusal));
    assert!(err.to_string().contains(refusal), "{err}");
}

/// A 401 means the passkey sign-in behind the token is too old (identity
/// wants one from the last hour). The owner can fix that; a retry cannot.
#[tokio::test]
async fn a_stale_sign_in_is_told_apart_from_a_refusal() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agents/authorize"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .mount(&server)
        .await;

    let err = client(&server)
        .authorize("owner-token", &market_delegation())
        .await
        .unwrap_err();

    assert!(
        matches!(err, AgentAuthError::OwnerSignInRejected(_)),
        "{err}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// An answer for a different agent, or one that is not `eligible`, is not the
/// authorization that was asked for.
#[tokio::test]
async fn an_answer_for_another_agent_is_not_taken_as_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agents/authorize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "success": true,
            "message": "Agent authorized successfully",
            "agent": "did:key:z6MkOther",
            "state": "eligible",
            "state_version": 1,
            "appraised_until": 1_790_043_200,
        })))
        .mount(&server)
        .await;

    let err = client(&server)
        .authorize("owner-token", &market_delegation())
        .await
        .unwrap_err();

    assert!(matches!(err, AgentAuthError::InvalidResponse(_)), "{err}");
}

/// The owner's revoke, as identity 0.37.0 words it: the owner's sign-in token
/// as a Bearer on `DELETE /agents/delegations/{did}`, answered 204.
#[tokio::test]
async fn revoke_presents_the_owners_sign_in_for_this_agent() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/agents/delegations/{AGENT}")))
        .and(header("authorization", "Bearer owner-token"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    client(&server).revoke("owner-token", AGENT).await.unwrap();
}

/// Only the owner may revoke. Identity's contract does not pin the body of
/// that refusal (this one is a stand-in), so whatever it says reaches the
/// owner unchanged.
#[tokio::test]
async fn a_refused_revoke_is_reported_in_identity_words() {
    let server = MockServer::start().await;
    let refusal = "Forbidden";
    Mock::given(method("DELETE"))
        .and(path(format!("/agents/delegations/{AGENT}")))
        .respond_with(ResponseTemplate::new(403).set_body_string(refusal))
        .mount(&server)
        .await;

    let err = client(&server)
        .revoke("owner-token", AGENT)
        .await
        .unwrap_err();

    assert!(matches!(&err, AgentAuthError::RevokeRefused(text) if text == refusal));
    assert!(err.to_string().contains(refusal), "{err}");
}

/// Revoking wants the same fresh passkey sign-in as authorizing, and a retry
/// cannot supply one.
#[tokio::test]
async fn a_stale_sign_in_stops_a_revoke_without_a_retry() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/agents/delegations/{AGENT}")))
        .respond_with(ResponseTemplate::new(401).set_body_string("Invalid token"))
        .mount(&server)
        .await;
    let client =
        AgentAuthClient::with_config(AgentAuthConfig::new(server.uri()).with_max_retries(3))
            .unwrap();

    let err = client.revoke("owner-token", AGENT).await.unwrap_err();

    assert!(
        matches!(err, AgentAuthError::OwnerSignInRejected(_)),
        "{err}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// The DID becomes a path segment, so anything that is not a plain `did:key`
/// is refused before identity is called.
#[tokio::test]
async fn a_did_that_would_change_the_path_is_never_sent() {
    let server = MockServer::start().await;

    for did in [
        "",
        "did:web:example.com",
        "did:key:z6Mk/../../authorize",
        "did:key:z6Mk?x=1",
        "did:key:z6Mk abc",
    ] {
        let err = client(&server)
            .revoke("owner-token", did)
            .await
            .unwrap_err();
        assert!(
            matches!(err, AgentAuthError::RevokeRequest(_)),
            "{did:?}: {err}"
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}
