mod client;
mod config;
mod error;
mod storage;
mod types;

pub use client::AgentAuthClient;
pub use config::AgentAuthConfig;
pub use error::AgentAuthError;
pub use storage::{delete_token, load_token, store_token};
pub use types::{ChallengeResponse, StoredToken, TokenRequest, TokenResponse};

#[cfg(test)]
pub(crate) mod test_helpers {
    use tokio::sync::Mutex;

    /// Serializes tests that touch the on-disk token file.
    pub(crate) static TEST_LOCK: Mutex<()> = Mutex::const_new(());
}
