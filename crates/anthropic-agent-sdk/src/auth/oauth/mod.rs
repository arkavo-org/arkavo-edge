//! OAuth 2.0 client with PKCE support for Claude authentication

mod pkce;
mod token_exchange;

use super::token::{TokenError, TokenInfo, TokenStorage};
use serde::Deserialize;
use thiserror::Error;

use pkce::PkceChallenge;

// Claude OAuth configuration (Claude Code's official client_id)
const DEFAULT_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const DEFAULT_AUTH_URL: &str = "https://claude.ai/oauth/authorize";
const DEFAULT_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const DEFAULT_REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const DEFAULT_SCOPES: &str = "user:profile user:inference";

/// Errors that can occur during OAuth operations
#[derive(Debug, Error)]
pub enum OAuthError {
    /// HTTP request failed
    #[error("HTTP request failed: {0}")]
    Http(String),

    /// Token exchange failed
    #[error("Token exchange failed: {0}")]
    TokenExchange(String),

    /// Invalid response from server
    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    /// User cancelled authentication
    #[error("Authentication cancelled by user")]
    Cancelled,

    /// Token storage error
    #[error("Token storage error: {0}")]
    Storage(#[from] TokenError),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON error
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Browser could not be opened
    #[error("Could not open browser: {0}")]
    BrowserOpen(String),

    /// HTTP client error
    #[error("HTTP client error: {0}")]
    Reqwest(#[from] reqwest::Error),
}

/// Result type for OAuth operations
pub type AuthResult<T> = Result<T, OAuthError>;

/// OAuth configuration
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// OAuth client ID
    pub client_id: String,
    /// Authorization endpoint URL
    pub auth_url: String,
    /// Token endpoint URL
    pub token_url: String,
    /// Redirect URI for OAuth callback
    pub redirect_uri: String,
    /// Space-separated scopes to request
    pub scopes: String,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            client_id: DEFAULT_CLIENT_ID.to_string(),
            auth_url: DEFAULT_AUTH_URL.to_string(),
            token_url: DEFAULT_TOKEN_URL.to_string(),
            redirect_uri: DEFAULT_REDIRECT_URI.to_string(),
            scopes: DEFAULT_SCOPES.to_string(),
        }
    }
}

/// OAuth response from token endpoint
#[derive(Debug, Deserialize)]
pub(super) struct TokenResponse {
    pub(super) access_token: String,
    #[serde(default)]
    pub(super) refresh_token: Option<String>,
    #[serde(default, rename = "token_type")]
    pub(super) _token_type: Option<String>,
    #[serde(default)]
    pub(super) expires_in: Option<u64>,
    #[serde(default)]
    pub(super) scope: Option<String>,
}

/// Error response from token endpoint
#[derive(Debug, Deserialize)]
pub(super) struct ErrorResponse {
    pub(super) error: String,
    #[serde(default)]
    pub(super) error_description: Option<String>,
}

/// Builder for [`OAuthClient`]
#[derive(Debug, Default)]
pub struct OAuthClientBuilder {
    config: Option<OAuthConfig>,
    storage: Option<TokenStorage>,
    auto_open_browser: bool,
}

impl OAuthClientBuilder {
    /// Create a new builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: None,
            storage: None,
            auto_open_browser: true,
        }
    }

    /// Set custom OAuth configuration
    #[must_use]
    pub fn config(mut self, config: OAuthConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Set custom token storage
    #[must_use]
    pub fn storage(mut self, storage: TokenStorage) -> Self {
        self.storage = Some(storage);
        self
    }

    /// Set whether to automatically open browser (default: true)
    #[must_use]
    pub fn auto_open_browser(mut self, auto_open: bool) -> Self {
        self.auto_open_browser = auto_open;
        self
    }

    /// Build the OAuth client
    #[must_use]
    pub fn build(self) -> OAuthClient {
        OAuthClient {
            config: self.config.unwrap_or_default(),
            storage: self.storage.unwrap_or_default(),
            auto_open_browser: self.auto_open_browser,
            http_client: reqwest::Client::new(),
        }
    }
}

/// OAuth client for Claude authentication
#[derive(Debug)]
pub struct OAuthClient {
    pub(super) config: OAuthConfig,
    pub(super) storage: TokenStorage,
    pub(super) auto_open_browser: bool,
    pub(super) http_client: reqwest::Client,
}

impl OAuthClient {
    /// Create a new OAuth client with default configuration
    ///
    /// # Errors
    ///
    /// Returns an error if token storage initialization fails.
    pub fn new() -> AuthResult<Self> {
        Ok(Self {
            config: OAuthConfig::default(),
            storage: TokenStorage::new(),
            auto_open_browser: true,
            http_client: reqwest::Client::new(),
        })
    }

    /// Create a builder for custom configuration
    #[must_use]
    pub fn builder() -> OAuthClientBuilder {
        OAuthClientBuilder::new()
    }

    /// Get the OAuth configuration
    #[must_use]
    pub fn config(&self) -> &OAuthConfig {
        &self.config
    }

    /// Get the token storage
    #[must_use]
    pub fn storage(&self) -> &TokenStorage {
        &self.storage
    }

    /// Authenticate - try cached token first, then OAuth flow
    ///
    /// # Errors
    ///
    /// Returns an error if authentication fails (network, user cancellation, etc.)
    pub async fn authenticate(&self) -> AuthResult<TokenInfo> {
        // Try to load cached valid token
        match self.storage.load_valid() {
            Ok(token) => {
                tracing::debug!("Using cached OAuth token");
                return Ok(token);
            }
            Err(TokenError::Expired) => {
                // Try to refresh if we have a refresh token
                if let Ok(old_token) = self.storage.load()
                    && let Some(ref refresh_token) = old_token.refresh_token
                {
                    tracing::debug!("Attempting token refresh");
                    match self.refresh_token(refresh_token).await {
                        Ok(new_token) => return Ok(new_token),
                        Err(e) => {
                            tracing::warn!("Token refresh failed: {e}");
                            // Fall through to full OAuth flow
                        }
                    }
                }
            }
            Err(TokenError::NotFound) => {
                tracing::debug!("No cached token found");
            }
            Err(e) => {
                tracing::warn!("Error loading cached token: {e}");
            }
        }

        // Start full OAuth flow
        self.start_oauth_flow().await
    }

    /// Start the OAuth authorization flow
    ///
    /// # Errors
    ///
    /// Returns an error if the OAuth flow fails.
    pub async fn start_oauth_flow(&self) -> AuthResult<TokenInfo> {
        let pkce = PkceChallenge::generate();

        // Build authorization URL
        let auth_url = self.build_auth_url(&pkce.challenge);

        println!("\n🔐 Claude OAuth Authentication");
        println!(
            "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
        );
        println!();
        println!("To authenticate, please:");
        println!("  1. Open the following URL in your browser");
        println!("  2. Log in with your Anthropic account");
        println!("  3. Copy the authorization code shown after approval");
        println!("  4. Paste the code below");
        println!();
        println!("Authorization URL:");
        println!("  {auth_url}");
        println!();

        // Try to open browser automatically
        if self.auto_open_browser {
            if let Err(e) = Self::open_browser(&auth_url) {
                tracing::debug!("Could not open browser: {e}");
                println!("(Could not open browser automatically - please open the URL manually)");
            } else {
                println!("(Opening browser...)");
            }
        }

        println!();

        // Read authorization code from user
        let (code, state) = Self::prompt_for_code()?;

        if code.is_empty() || code.to_lowercase() == "cancel" {
            return Err(OAuthError::Cancelled);
        }

        // Exchange code for token
        let token = self
            .exchange_code(&code, state.as_deref(), &pkce.verifier)
            .await?;

        // Save token
        self.storage.save(&token)?;
        println!();
        println!("✓ Authentication successful! Token cached at:");
        println!("  {}", self.storage.path().display());

        Ok(token)
    }

    /// Log out - delete cached token
    ///
    /// # Errors
    ///
    /// Returns an error if token deletion fails.
    pub fn logout(&self) -> AuthResult<()> {
        self.storage.delete()?;
        println!("✓ Logged out successfully");
        Ok(())
    }

    /// Check if user is authenticated (has valid token)
    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.storage.has_valid_token()
    }

    /// Get current token without refreshing
    #[must_use]
    pub fn current_token(&self) -> Option<TokenInfo> {
        self.storage.load().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pkce::urlencoding;

    #[test]
    fn test_pkce_challenge_generation() {
        let pkce = PkceChallenge::generate();
        assert!(!pkce.verifier.is_empty());
        assert!(!pkce.challenge.is_empty());
        // Verifier should be 43 chars (32 bytes base64url encoded)
        assert_eq!(pkce.verifier.len(), 43);
        // Challenge should be 43 chars (32 bytes SHA256 base64url encoded)
        assert_eq!(pkce.challenge.len(), 43);
    }

    #[test]
    fn test_pkce_verifier_is_valid_base64url() {
        let pkce = PkceChallenge::generate();
        // Should only contain base64url characters
        assert!(
            pkce.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert!(
            pkce.challenge
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding("hello"), "hello");
        assert_eq!(urlencoding("hello world"), "hello%20world");
        assert_eq!(urlencoding("a+b"), "a%2Bb");
        assert_eq!(urlencoding("user:profile"), "user%3Aprofile");
        assert_eq!(
            urlencoding("https://example.com"),
            "https%3A%2F%2Fexample.com"
        );
    }

    #[test]
    fn test_oauth_config_default() {
        let config = OAuthConfig::default();
        assert_eq!(config.client_id, DEFAULT_CLIENT_ID);
        assert_eq!(config.auth_url, DEFAULT_AUTH_URL);
        assert_eq!(config.token_url, DEFAULT_TOKEN_URL);
    }

    #[test]
    fn test_oauth_client_builder() {
        let client = OAuthClient::builder().auto_open_browser(false).build();

        assert!(!client.auto_open_browser);
        assert_eq!(client.config.client_id, DEFAULT_CLIENT_ID);
    }
}
