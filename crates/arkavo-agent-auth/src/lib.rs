mod client;
mod config;
mod error;
mod refresh;
mod storage;
mod types;

pub use client::AgentAuthClient;
pub use config::AgentAuthConfig;
pub use error::AgentAuthError;
pub use refresh::{RefreshState, run_refresh_loop};
pub use storage::{delete_token, load_token, store_token};
pub use types::{ChallengeResponse, StoredToken, TokenRequest, TokenResponse};

#[cfg(test)]
pub(crate) mod test_helpers {
    use crate::types::{TokenRequest, TokenResponse};
    use arkavo_crypto::AgentPublicKey;
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use tokio::sync::Mutex;
    use wiremock::{Request, Respond, ResponseTemplate};

    /// Serializes tests that touch the on-disk token file or process
    /// environment variables.
    pub(crate) static TEST_LOCK: Mutex<()> = Mutex::const_new(());

    /// Wiremock responder that validates the Ed25519 signature on the token
    /// request before returning a token response. Shared by `client.rs` and
    /// `refresh.rs` tests, which both need a server that only issues a token
    /// once a correctly-signed challenge comes back.
    pub(crate) struct ValidatingTokenResponder {
        public_key: AgentPublicKey,
        challenge: Vec<u8>,
    }

    impl ValidatingTokenResponder {
        pub(crate) fn new(public_key: AgentPublicKey, challenge: Vec<u8>) -> Self {
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
                expires_at: chrono::Utc::now() + chrono::Duration::minutes(15),
                entitlements: vec!["https://arkavo.ai/attr/tdf/value/decrypt".to_string()],
            })
        }
    }
}
