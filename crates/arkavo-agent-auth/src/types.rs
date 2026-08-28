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
///
/// `expires_at` is a unix-epoch-seconds integer on the wire (authnz-rs encodes
/// it that way), not an RFC3339 string, so it needs an explicit chrono serde
/// adapter distinct from `StoredToken`'s default (RFC3339) representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub token: String,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub expires_at: DateTime<Utc>,
    #[serde(default)]
    pub entitlements: Vec<String>,
}

/// Token stored locally on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredToken {
    pub token: String,
    pub did: String,
    pub expires_at: DateTime<Utc>,
    pub entitlements: Vec<String>,
    pub stored_at: DateTime<Utc>,
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
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    /// True once two-thirds of the token's lifetime (`stored_at` to
    /// `expires_at`) has elapsed. There are no refresh tokens in this design;
    /// "refresh" means re-running the challenge-response flow before the
    /// short-lived CWT actually expires.
    pub fn needs_refresh(&self, now: DateTime<Utc>) -> bool {
        let two_thirds = self.stored_at + (self.expires_at - self.stored_at) * 2 / 3;
        now >= two_thirds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[test]
    #[spec("AAUTH-004")]
    fn needs_refresh_at_two_thirds_lifetime() {
        let t0 = Utc::now();
        let mut tok = StoredToken::new(
            "t".into(),
            "did".into(),
            t0 + chrono::Duration::minutes(15),
            vec![],
        );
        tok.stored_at = t0;
        assert!(!tok.needs_refresh(t0 + chrono::Duration::minutes(9)));
        assert!(tok.needs_refresh(t0 + chrono::Duration::minutes(10)));
        assert!(tok.needs_refresh(t0 + chrono::Duration::minutes(16)));
    }

    /// Regression test for C1: authnz-rs sends `expires_at` as an integer
    /// unix-epoch-seconds value, not an RFC3339 string. A round-trip test that
    /// serializes and deserializes the same struct (as the old wiremock
    /// responder did) can't catch this, because any encoding round-trips.
    /// This test deserializes a literal JSON fixture shaped like the real
    /// server response instead, and also carries an unrelated unknown field
    /// to confirm serde's default unknown-field tolerance holds now that the
    /// response no longer carries a delegation JWT (removed per C2).
    #[test]
    #[spec("AAUTH-004")]
    fn token_response_deserializes_integer_expires_at() {
        let json = r#"{
            "token": "opaque-cwt-bytes",
            "expires_at": 1788000000,
            "entitlements": ["https://arkavo.ai/attr/tdf/value/decrypt"],
            "future_unknown_field": "should-be-ignored"
        }"#;

        let parsed: TokenResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.token, "opaque-cwt-bytes");
        assert_eq!(parsed.expires_at.timestamp(), 1_788_000_000);
        assert_eq!(
            parsed.entitlements,
            vec!["https://arkavo.ai/attr/tdf/value/decrypt".to_string()]
        );
    }
}
