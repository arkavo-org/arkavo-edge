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
