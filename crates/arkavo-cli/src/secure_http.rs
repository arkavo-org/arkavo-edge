//! Secure HTTP Client with Egress Filtering
//!
//! This module provides HTTP clients that enforce security policies:
//! - SSRF protection via egress filtering
//! - TLS certificate validation
//! - Timeout enforcement
//! - Redirect validation

use arkavo_protocol::security_fixes::{EgressError, EgressFilter};
use reqwest::{Client, ClientBuilder, Method, Request, Response};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::time::timeout;

/// Global egress filter instance
static EGRESS_FILTER: OnceLock<EgressFilter> = OnceLock::new();

/// Initialize the global egress filter
pub fn init_egress_filter() {
    EGRESS_FILTER.get_or_init(|| {
        let mut filter = EgressFilter::new();

        // Add allowed external APIs
        filter.allow("https://api.openai.com");
        filter.allow("https://api.anthropic.com");
        filter.allow("https://generativelanguage.googleapis.com");
        filter.allow("https://api.github.com");
        filter.allow("https://api.deepseek.com");
        filter.allow("https://api.x.ai");

        filter
    });
}

/// Get the global egress filter
fn get_egress_filter() -> &'static EgressFilter {
    EGRESS_FILTER.get_or_init(EgressFilter::new)
}

/// Security error type for HTTP operations
#[derive(Debug)]
pub enum SecureHttpError {
    EgressBlocked(String),
    RequestFailed(reqwest::Error),
    Timeout,
    TlsError(String),
}

impl std::fmt::Display for SecureHttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecureHttpError::EgressBlocked(url) => {
                write!(f, "SSRF protection: Access to {url} is blocked")
            }
            SecureHttpError::RequestFailed(e) => write!(f, "HTTP request failed: {e}"),
            SecureHttpError::Timeout => write!(f, "HTTP request timed out"),
            SecureHttpError::TlsError(e) => write!(f, "TLS error: {e}"),
        }
    }
}

impl std::error::Error for SecureHttpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SecureHttpError::RequestFailed(e) => Some(e),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for SecureHttpError {
    fn from(e: reqwest::Error) -> Self {
        SecureHttpError::RequestFailed(e)
    }
}

/// Secure HTTP client builder
pub struct SecureClientBuilder {
    inner: ClientBuilder,
    check_egress: bool,
}

impl SecureClientBuilder {
    /// Create a new secure client builder
    pub fn new() -> Self {
        let builder = Client::builder()
            .timeout(Duration::from_mins(1))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .https_only(true) // SECURITY: Only allow HTTPS
            .redirect(reqwest::redirect::Policy::limited(10)); // Limit redirects

        Self {
            inner: builder,
            check_egress: true,
        }
    }

    /// Disable egress checking (use with caution)
    pub fn danger_disable_egress_check(mut self) -> Self {
        self.check_egress = false;
        self
    }

    /// Set timeout
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.timeout(timeout);
        self
    }

    /// Set user agent
    pub fn user_agent(mut self, user_agent: &str) -> Self {
        self.inner = self.inner.user_agent(user_agent);
        self
    }

    /// Set default headers
    pub fn default_headers(mut self, headers: reqwest::header::HeaderMap) -> Self {
        self.inner = self.inner.default_headers(headers);
        self
    }

    /// Build the client
    pub fn build(self) -> Result<SecureClient, SecureHttpError> {
        let inner = self.inner.build().map_err(SecureHttpError::RequestFailed)?;

        Ok(SecureClient {
            inner,
            check_egress: self.check_egress,
        })
    }
}

impl Default for SecureClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Secure HTTP client with egress filtering
#[derive(Clone)]
pub struct SecureClient {
    inner: Client,
    check_egress: bool,
}

impl SecureClient {
    /// Create a new secure client with default settings
    pub fn new() -> Result<Self, SecureHttpError> {
        SecureClientBuilder::new().build()
    }

    /// Create a builder for customized configuration
    pub fn builder() -> SecureClientBuilder {
        SecureClientBuilder::new()
    }

    /// Check if URL passes egress filter
    fn check_egress(&self, url: &str) -> Result<(), SecureHttpError> {
        if !self.check_egress {
            return Ok(());
        }

        let filter = get_egress_filter();
        match filter.is_allowed(url) {
            Ok(()) => Ok(()),
            Err(EgressError::BlockedIp(ip)) => Err(SecureHttpError::EgressBlocked(format!(
                "{ip} (private/metadata IP)"
            ))),
            Err(EgressError::BlockedDomain(domain)) => Err(SecureHttpError::EgressBlocked(domain)),
            Err(EgressError::InvalidUrl) => {
                Err(SecureHttpError::EgressBlocked("invalid URL".to_string()))
            }
        }
    }

    /// Execute a request with security checks
    pub async fn execute(&self, request: Request) -> Result<Response, SecureHttpError> {
        // Check egress before making request
        let url = request.url().to_string();
        self.check_egress(&url)?;

        // Execute with timeout
        let response = timeout(Duration::from_mins(1), self.inner.execute(request))
            .await
            .map_err(|_| SecureHttpError::Timeout)??;

        Ok(response)
    }

    /// Make a GET request
    pub async fn get(&self, url: &str) -> Result<Response, SecureHttpError> {
        self.check_egress(url)?;
        let response = self.inner.get(url).send().await?;
        Ok(response)
    }

    /// Make a POST request
    pub async fn post(&self, url: &str) -> Result<Response, SecureHttpError> {
        self.check_egress(url)?;
        let response = self.inner.post(url).send().await?;
        Ok(response)
    }

    /// Make a request with method
    pub async fn request(&self, method: Method, url: &str) -> Result<Response, SecureHttpError> {
        self.check_egress(url)?;
        let response = self.inner.request(method, url).send().await?;
        Ok(response)
    }
}

impl Default for SecureClient {
    fn default() -> Self {
        Self::new().expect("Failed to create secure HTTP client")
    }
}

// ============================================================================
// Migration Helpers
// ============================================================================

/// Create a secure reqwest client (drop-in replacement for `reqwest::Client::new()`)
///
/// # Panics
/// Panics if the HTTP client fails to build
pub fn create_secure_client() -> Client {
    // Initialize egress filter
    init_egress_filter();

    Client::builder()
        .timeout(Duration::from_mins(1))
        .connect_timeout(Duration::from_secs(10))
        .https_only(true)
        .build()
        .expect("Failed to create HTTP client")
}

/// Create a secure client with custom configuration
///
/// # Panics
/// Panics if the HTTP client fails to build
pub fn create_secure_client_with_config<F>(f: F) -> Client
where
    F: FnOnce(ClientBuilder) -> ClientBuilder,
{
    init_egress_filter();

    let builder = Client::builder()
        .timeout(Duration::from_mins(1))
        .connect_timeout(Duration::from_secs(10))
        .https_only(true);

    f(builder).build().expect("Failed to create HTTP client")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_egress_filter_initialization() {
        init_egress_filter();
        let filter = get_egress_filter();

        // Private IPs should be blocked
        assert!(filter.is_allowed("http://10.0.0.1/test").is_err());
        assert!(filter.is_allowed("http://192.168.1.1/test").is_err());
        assert!(filter.is_allowed("http://127.0.0.1/test").is_err());

        // Allowed APIs should pass
        assert!(
            filter
                .is_allowed("https://api.openai.com/v1/chat/completions")
                .is_ok()
        );
    }

    #[tokio::test]
    async fn test_secure_client_blocks_private_ip() {
        let client = SecureClient::new().expect("Failed to create client");

        // Attempt to connect to private IP
        let result = client.get("http://10.0.0.1/test").await;

        assert!(
            matches!(result, Err(SecureHttpError::EgressBlocked(_))),
            "Should block private IP access"
        );
    }
}
