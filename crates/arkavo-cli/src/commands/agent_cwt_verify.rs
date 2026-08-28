//! The agent validating its own credential on receipt.
//!
//! The CWT is issued by authnz-rs and used as a bearer token against the KAS.
//! Verifying it here turns a credential the edge would otherwise treat as an
//! opaque string into one it has actually checked: right issuer, live ES256
//! signature from the issuer's published key set, not expired.

use arkavo_cwt::{CachedKeySet, VerifyOptions};
use std::sync::OnceLock;
use std::time::Duration;

/// authnz-rs's own `DEFAULT_OIDC_ISSUER`.
const DEFAULT_ISSUER: &str = "https://identity.arkavo.net";

/// Matches the `Cache-Control: max-age=600` the cose-keys endpoint sends.
const KEY_SET_TTL: Duration = Duration::from_secs(600);

/// Allowance for clock drift between the edge and the issuer.
const SKEW_SECS: i64 = 60;

/// Verify the agent's freshly stored CWT and warn if it does not check out.
///
/// Warn-only by design: this is a diagnostic, not an authorization decision.
/// A verifier bug, an unreachable key-set endpoint, or an issuer the edge is
/// not configured for must not stop the agent from using a token the server
/// just issued.
pub(crate) async fn verify_stored_token() {
    let token = match arkavo_agent_auth::load_token().await {
        Ok(Some(token)) => token,
        // The refresh loop stores before it reports Authenticated, so an empty
        // slot here means the token was dropped again in between; nothing to do.
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(error = %e, "could not load the agent CWT for self-verification");
            return;
        }
    };

    let issuer = std::env::var("ARKAVO_AUTH_ISSUER").unwrap_or_else(|_| DEFAULT_ISSUER.to_string());
    let opts = VerifyOptions {
        expected_iss: &issuer,
        // Nothing on the edge knows the issuer's configured AGENT_TOKEN_AUDIENCES.
        expected_aud: None,
        now: chrono::Utc::now().timestamp(),
        skew_secs: SKEW_SECS,
    };

    match key_set().verify(&token.token, &opts).await {
        Ok(claims) => {
            tracing::debug!(
                sub = %claims.sub,
                exp = claims.exp,
                entitlements = claims.entitlements.len(),
                "agent CWT verified against the issuer's COSE key set"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, issuer = %issuer, "agent CWT failed self-verification");
        }
    }
}

/// The process-wide key-set cache, so repeated verifications share one fetch.
fn key_set() -> &'static CachedKeySet {
    static KEYS: OnceLock<CachedKeySet> = OnceLock::new();
    KEYS.get_or_init(|| {
        let base = arkavo_agent_auth::AgentAuthConfig::from_env().base_url;
        CachedKeySet::new(
            format!("{}/.well-known/cose-keys", base.trim_end_matches('/')),
            KEY_SET_TTL,
        )
    })
}
