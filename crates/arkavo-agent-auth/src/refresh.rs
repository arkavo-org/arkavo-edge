use crate::{client::AgentAuthClient, error::AgentAuthError};
use arkavo_crypto::AgentKeypair;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use std::time::Duration;

/// Observable state of the agent's identity plane, driven by
/// [`run_refresh_loop`]. There are no refresh tokens in this design: staying
/// authenticated means re-running the DID challenge-response before the
/// short-lived CWT actually expires.
#[derive(Debug, Clone)]
pub enum RefreshState {
    /// No delegation row yet; the human has not approved this agent's DID.
    WaitingForApproval,
    /// Holding a valid CWT, expiring at the given time.
    Authenticated { expires_at: DateTime<Utc> },
    /// The delegation was revoked (or the token expired past recovery); the
    /// stored token has already been dropped.
    Revoked,
}

/// Poll `client.get_token` on a fixed interval and report state transitions
/// through `on_state`. `get_token` is a no-op once a token is cached and not
/// yet within its last third of lifetime, so polling faster than the token's
/// own refresh cadence costs nothing beyond a disk read.
///
/// Transient errors (network failures, 5xx) don't change the reported state;
/// they're logged and the loop keeps polling rather than flapping the UI
/// between "authenticated" and some transient-failure state.
pub async fn run_refresh_loop(
    client: Arc<AgentAuthClient>,
    keypair: Arc<AgentKeypair>,
    poll: Duration,
    on_state: impl Fn(RefreshState) + Send + Sync + 'static,
) {
    let mut last: Option<RefreshState> = None;
    loop {
        let next = match client.get_token(&keypair).await {
            Ok(tok) => RefreshState::Authenticated {
                expires_at: tok.expires_at,
            },
            Err(AgentAuthError::NotAuthorized) => RefreshState::WaitingForApproval,
            Err(AgentAuthError::Forbidden(_)) => RefreshState::Revoked,
            Err(e) => {
                tracing::warn!(error = %e, "agent token refresh failed; will retry");
                last.clone().unwrap_or(RefreshState::WaitingForApproval)
            }
        };

        if last.as_ref().map(std::mem::discriminant) != Some(std::mem::discriminant(&next)) {
            on_state(next.clone());
        }
        last = Some(next);

        tokio::time::sleep(poll).await;
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::config::AgentAuthConfig;
    use crate::storage;
    use crate::test_helpers::{TEST_LOCK, ValidatingTokenResponder};
    use crate::types::ChallengeResponse;
    use arkavo_test_macros::spec;
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use std::sync::Mutex;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    #[spec("AAUTH-004")]
    async fn refresh_loop_reports_waiting_then_authenticated() {
        let _g = TEST_LOCK.lock().await;
        storage::delete_token().await.unwrap();

        let server = MockServer::start().await;
        let keypair = AgentKeypair::generate();
        let challenge = b"refresh-loop-challenge".to_vec();
        let challenge_b64 = BASE64_STANDARD.encode(&challenge);

        // First poll: no delegation row yet.
        Mock::given(method("GET"))
            .and(path("/agents/challenge"))
            .respond_with(ResponseTemplate::new(404))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // Subsequent polls: the human has approved; the challenge succeeds.
        Mock::given(method("GET"))
            .and(path("/agents/challenge"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ChallengeResponse {
                challenge: challenge_b64,
                nonce: "nonce-refresh".to_string(),
            }))
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/agents/token"))
            .respond_with(ValidatingTokenResponder::new(
                keypair.public_key(),
                challenge,
            ))
            .mount(&server)
            .await;

        let client =
            Arc::new(AgentAuthClient::with_config(AgentAuthConfig::new(server.uri())).unwrap());
        let keypair = Arc::new(keypair);

        let states = Arc::new(Mutex::new(Vec::new()));
        let states_writer = states.clone();
        let handle = tokio::spawn(run_refresh_loop(
            client,
            keypair,
            Duration::from_millis(20),
            move |state| states_writer.lock().unwrap().push(state),
        ));

        tokio::time::sleep(Duration::from_millis(300)).await;
        handle.abort();

        {
            let seen = states.lock().unwrap();
            assert!(!seen.is_empty(), "expected at least one reported state");
            assert!(matches!(seen[0], RefreshState::WaitingForApproval));
            assert!(
                seen.iter()
                    .any(|s| matches!(s, RefreshState::Authenticated { .. })),
                "expected the loop to reach Authenticated after approval, saw {seen:?}"
            );
        }

        storage::delete_token().await.unwrap();
    }
}
