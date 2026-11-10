use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Response from the challenge endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChallengeResponse {
    pub challenge: String,
    pub nonce: String,
}

/// Request body for the token endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRequest {
    pub did: String,
    pub challenge: String,
    pub signature: String,
    pub nonce: String,
}

/// Response from the token endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub token: String,
    /// Unix seconds on the wire, as authnz-rs sends it.
    #[serde(with = "chrono::serde::ts_seconds")]
    pub expires_at: DateTime<Utc>,
    #[serde(default)]
    pub entitlements: Vec<String>,
    /// ES256-signed delegation JWT from authnz-rs binding human DID → agent DID
    #[serde(default)]
    pub delegation_jwt: Option<String>,
}

/// Token stored locally on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub token: String,
    pub did: String,
    pub expires_at: DateTime<Utc>,
    pub entitlements: Vec<String>,
    pub stored_at: DateTime<Utc>,
    /// ES256-signed delegation JWT binding human DID → agent DID
    #[serde(default)]
    pub delegation_jwt: Option<String>,
}

impl StoredToken {
    pub fn new(
        token: String,
        did: String,
        expires_at: DateTime<Utc>,
        entitlements: Vec<String>,
    ) -> Self {
        Self {
            token,
            did,
            expires_at,
            entitlements,
            stored_at: Utc::now(),
            delegation_jwt: None,
        }
    }

    pub fn with_delegation_jwt(mut self, jwt: Option<String>) -> Self {
        self.delegation_jwt = jwt;
        self
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }
}

/// What an owner lets one agent do: the body of identity's
/// `POST /agents/authorize`.
///
/// An authorize replaces whatever the agent was delegated before, so
/// `entitlements` is the whole of what it may carry afterwards. Identity
/// refuses the call when the owner does not hold one of them.
#[derive(Debug, Clone, Serialize)]
pub struct Delegation {
    /// The agent's `did:key`.
    pub agent_did: String,
    /// A label for people; not an identifier.
    pub name: String,
    pub entitlements: Vec<String>,
    /// Caps each token minted under this delegation at five minutes.
    pub short_lived: bool,
}

/// Identity's answer to an authorize it accepted.
#[derive(Debug, Clone)]
pub struct Authorized {
    /// The agent can mint tokens until then. After it the owner authorizes
    /// again, with a fresh passkey sign-in.
    pub appraised_until: DateTime<Utc>,
}

/// The wire form of [`Authorized`], with the fields that are checked and
/// not kept.
#[derive(Debug, Deserialize)]
pub(crate) struct AuthorizeResponse {
    pub(crate) agent: String,
    pub(crate) state: String,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub(crate) appraised_until: DateTime<Utc>,
}
