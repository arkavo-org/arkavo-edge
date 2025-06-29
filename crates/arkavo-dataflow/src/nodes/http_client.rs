use anyhow::Result;
use reqwest::{Client, ClientBuilder};
use std::sync::Arc;
use std::time::Duration;

/// Configuration for HTTP client
#[derive(Clone, Debug)]
pub struct HttpClientConfig {
    /// Base URL for the service
    pub base_url: String,
    /// Optional authentication token
    pub auth_token: Option<String>,
    /// Optional custom root certificates
    pub root_certificates: Option<Vec<Vec<u8>>>,
    /// Connection timeout in seconds
    pub timeout_secs: u64,
    /// Maximum number of retries
    pub max_retries: u32,
    /// Initial retry delay in milliseconds
    pub initial_retry_delay_ms: u64,
    /// Backoff factor for exponential retry
    pub backoff_factor: f64,
    /// Maximum retry delay in milliseconds
    pub max_retry_delay_ms: u64,
    /// Jitter factor (0.0 to 1.0)
    pub jitter_factor: f64,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            auth_token: None,
            root_certificates: None,
            timeout_secs: 30,
            max_retries: 3,
            initial_retry_delay_ms: 100,
            backoff_factor: 2.0,
            max_retry_delay_ms: 10000,
            jitter_factor: 0.1,
        }
    }
}

/// Shared HTTP client builder for all providers
pub struct HttpClientBuilder {
    config: HttpClientConfig,
}

impl HttpClientBuilder {
    pub fn new(config: HttpClientConfig) -> Self {
        Self { config }
    }

    /// Build a configured HTTP client
    pub fn build(&self) -> Result<Client> {
        let mut builder = ClientBuilder::new()
            .timeout(Duration::from_secs(self.config.timeout_secs))
            .use_rustls_tls() // Use rustls instead of native TLS
            .pool_idle_timeout(Duration::from_secs(90))
            .pool_max_idle_per_host(10);

        // Add custom root certificates if provided
        if let Some(ref certs) = self.config.root_certificates {
            for cert_der in certs {
                let cert = reqwest::Certificate::from_der(cert_der)?;
                builder = builder.add_root_certificate(cert);
            }
        }

        // Add default headers
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::USER_AGENT,
            reqwest::header::HeaderValue::from_static("arkavo-edge/0.19.0"),
        );

        // Add authorization header if token is provided
        if let Some(ref token) = self.config.auth_token {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))?,
            );
        }

        builder = builder.default_headers(headers);

        Ok(builder.build()?)
    }

    /// Calculate retry delay with exponential backoff and jitter
    pub fn calculate_retry_delay(&self, attempt: u32) -> Duration {
        let base_delay = self.config.initial_retry_delay_ms as f64
            * self
                .config
                .backoff_factor
                .powi(i32::try_from(attempt).unwrap_or(i32::MAX));

        let capped_delay = base_delay.min(self.config.max_retry_delay_ms as f64);

        // Add jitter
        let jitter_range = capped_delay * self.config.jitter_factor;
        let jitter = (rand::random::<f64>() * jitter_range).mul_add(2.0, -jitter_range);
        let final_delay = (capped_delay + jitter).max(0.0) as u64;

        Duration::from_millis(final_delay)
    }

    /// Get the maximum number of retries
    pub fn max_retries(&self) -> u32 {
        self.config.max_retries
    }
}

/// Shared HTTP client wrapper with retry logic
pub struct RetryableHttpClient {
    pub client: Client,
    builder: Arc<HttpClientBuilder>,
}

impl RetryableHttpClient {
    pub fn new(builder: HttpClientBuilder) -> Result<Self> {
        let client = builder.build()?;
        Ok(Self {
            client,
            builder: Arc::new(builder),
        })
    }

    /// Execute a request with retry logic
    pub async fn execute_with_retry<F, T>(&self, operation: F) -> Result<T>
    where
        F: Fn(&Client) -> futures::future::BoxFuture<'_, Result<T>>,
    {
        let mut last_error = None;

        for attempt in 0..=self.builder.max_retries() {
            match operation(&self.client).await {
                Ok(result) => return Ok(result),
                Err(err) => {
                    last_error = Some(err);

                    if attempt < self.builder.max_retries() {
                        let delay = self.builder.calculate_retry_delay(attempt);
                        tracing::warn!(
                            "Request failed (attempt {}/{}), retrying in {:?}",
                            attempt + 1,
                            self.builder.max_retries() + 1,
                            delay
                        );
                        tokio::time::sleep(delay).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("All retry attempts failed")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = HttpClientConfig::default();
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.backoff_factor, 2.0);
    }

    #[test]
    fn test_retry_delay_calculation() {
        let config = HttpClientConfig {
            initial_retry_delay_ms: 100,
            backoff_factor: 2.0,
            max_retry_delay_ms: 5000,
            jitter_factor: 0.0, // No jitter for predictable testing
            ..Default::default()
        };

        let builder = HttpClientBuilder::new(config);

        // First retry: 100ms
        assert_eq!(builder.calculate_retry_delay(0).as_millis(), 100);

        // Second retry: 200ms
        assert_eq!(builder.calculate_retry_delay(1).as_millis(), 200);

        // Third retry: 400ms
        assert_eq!(builder.calculate_retry_delay(2).as_millis(), 400);

        // Should be capped at max_retry_delay_ms
        assert_eq!(builder.calculate_retry_delay(10).as_millis(), 5000);
    }

    #[tokio::test]
    async fn test_client_builder() {
        let config = HttpClientConfig {
            base_url: "https://api.example.com".to_string(),
            auth_token: Some("test-token".to_string()),
            ..Default::default()
        };

        let builder = HttpClientBuilder::new(config);
        let client = builder.build();

        assert!(client.is_ok());
    }
}
