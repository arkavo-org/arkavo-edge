use crate::error::{A2aError, Result};
use crate::transport::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig};
use async_trait::async_trait;
use reqwest::{Certificate, Client, ClientBuilder, Identity};
use std::fs;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tracing::{debug, info, warn};

pub struct HttpTransport {
    config: TransportConfig,
    client: Client,
    endpoint: Arc<RwLock<Option<A2aEndpoint>>>,
}

impl HttpTransport {
    pub fn new(config: TransportConfig) -> Result<Self> {
        let timeout = Duration::from_millis(config.timeout_ms);

        let mut builder = ClientBuilder::new()
            .use_rustls_tls()
            .timeout(timeout)
            .connect_timeout(timeout);

        if !config.tls_config.verify_cert {
            builder = builder.danger_accept_invalid_certs(true);
        }

        // Load client certificates for mTLS
        if let (Some(cert_path), Some(key_path)) = (
            &config.tls_config.client_cert_path,
            &config.tls_config.client_key_path,
        ) {
            debug!("Loading client certificate from: {}", cert_path);
            debug!("Loading client key from: {}", key_path);

            let cert_pem = fs::read(cert_path)
                .map_err(|e| A2aError::Tls(format!("Failed to read client certificate: {e}")))?;
            let key_pem = fs::read(key_path)
                .map_err(|e| A2aError::Tls(format!("Failed to read client key: {e}")))?;

            // Combine certificate and key into a single PEM for reqwest
            let mut combined_pem = cert_pem;
            combined_pem.extend_from_slice(&key_pem);

            let identity = Identity::from_pem(&combined_pem)
                .map_err(|e| A2aError::Tls(format!("Failed to create identity from PEM: {e}")))?;

            builder = builder.identity(identity);
            info!("Client certificate configured for mTLS");
        }

        // Load CA certificate if provided
        if let Some(ca_path) = &config.tls_config.ca_cert_path {
            debug!("Loading CA certificate from: {}", ca_path);

            let ca_pem = fs::read(ca_path)
                .map_err(|e| A2aError::Tls(format!("Failed to read CA certificate: {e}")))?;

            let ca_cert = Certificate::from_pem(&ca_pem)
                .map_err(|e| A2aError::Tls(format!("Failed to parse CA certificate: {e}")))?;

            builder = builder.add_root_certificate(ca_cert);
            info!("CA certificate configured");
        }

        let client = builder
            .build()
            .map_err(|e| A2aError::Transport(format!("Failed to build HTTP client: {e}")))?;

        Ok(Self {
            config,
            client,
            endpoint: Arc::new(RwLock::new(None)),
        })
    }

    fn validate_endpoint(&self, endpoint: &A2aEndpoint) -> Result<()> {
        if !endpoint.url.starts_with("http://") && !endpoint.url.starts_with("https://") {
            return Err(A2aError::InvalidEndpoint(format!(
                "HTTP URL must start with http:// or https://, got: {}",
                endpoint.url
            )));
        }

        if self.config.tls_config.require_tls && endpoint.url.starts_with("http://") {
            return Err(A2aError::Tls(
                "TLS is required but URL uses http:// instead of https://".to_string(),
            ));
        }

        Ok(())
    }
}

#[async_trait]
impl A2aTransport for HttpTransport {
    async fn connect(&self, endpoint: &A2aEndpoint) -> anyhow::Result<()> {
        info!("Connecting to HTTP endpoint: {}", endpoint.url);

        self.validate_endpoint(endpoint)?;

        let health_check_url = format!("{}/health", endpoint.url.trim_end_matches('/'));
        let response = self.client.get(&health_check_url).send().await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                debug!("Health check successful for {}", endpoint.url);
            }
            Ok(resp) => {
                warn!(
                    "Health check returned status {} for {}",
                    resp.status(),
                    endpoint.url
                );
            }
            Err(e) => {
                debug!("Health check failed for {}: {}", endpoint.url, e);
            }
        }

        {
            let mut endpoint_guard = self.endpoint.write().unwrap();
            *endpoint_guard = Some(endpoint.clone());
        }

        info!("Successfully connected to {}", endpoint.url);
        Ok(())
    }

    async fn send_request(&self, request: A2aRequest) -> anyhow::Result<A2aResponse> {
        debug!(
            "Sending HTTP request: method={}, id={}",
            request.method, request.id
        );

        let endpoint = {
            let guard = self.endpoint.read().unwrap();
            guard.as_ref().ok_or(A2aError::NotConnected)?.clone()
        };

        let rpc_url = format!("{}/rpc", endpoint.url.trim_end_matches('/'));

        let mut retries = 0;
        let max_retries = self.config.max_retries;
        let retry_delay = Duration::from_millis(self.config.retry_delay_ms);

        loop {
            let response = self
                .client
                .post(&rpc_url)
                .json(&request)
                .header("Content-Type", "application/json")
                .header("X-Agent-ID", &endpoint.agent_id)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let response: A2aResponse = resp.json().await.map_err(|e| {
                        A2aError::InvalidResponse(format!("Failed to parse response: {e}"))
                    })?;

                    // Check if it's a retryable JSON-RPC error
                    if let A2aResponse::Error { error, .. } = &response {
                        // Retry on internal errors (-32603)
                        if error.code == -32603 && retries < max_retries {
                            warn!(
                                "Request returned internal error, retrying... ({}/{})",
                                retries + 1,
                                max_retries
                            );
                            retries += 1;
                            tokio::time::sleep(retry_delay).await;
                            continue;
                        }
                    }

                    debug!("Received response for request id={}", request.id);
                    return Ok(response);
                }
                Ok(resp) => {
                    let status = resp.status();
                    let error_text = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());

                    if status.as_u16() == 429 {
                        return Err(A2aError::RateLimitExceeded.into());
                    }

                    if retries < max_retries {
                        warn!(
                            "Request failed with status {}, retrying... ({}/{})",
                            status,
                            retries + 1,
                            max_retries
                        );
                        retries += 1;
                        tokio::time::sleep(retry_delay).await;
                        continue;
                    }

                    return Err(A2aError::Http(format!(
                        "Request failed with status {status}: {error_text}"
                    ))
                    .into());
                }
                Err(e) => {
                    if retries < max_retries {
                        warn!(
                            "Request failed: {}, retrying... ({}/{})",
                            e,
                            retries + 1,
                            max_retries
                        );
                        retries += 1;
                        tokio::time::sleep(retry_delay).await;
                        continue;
                    }

                    return Err(A2aError::Http(format!("Request failed: {e}")).into());
                }
            }
        }
    }

    async fn close(&self) -> anyhow::Result<()> {
        info!("Closing HTTP connection");

        {
            let mut endpoint_guard = self.endpoint.write().unwrap();
            *endpoint_guard = None;
        }

        Ok(())
    }

    fn is_connected(&self) -> bool {
        let guard = self.endpoint.read().unwrap();
        guard.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_transport_creation() {
        let config = TransportConfig::default();
        let transport = HttpTransport::new(config).unwrap();
        assert!(!transport.is_connected());
    }

    #[tokio::test]
    async fn test_invalid_endpoint() {
        let config = TransportConfig::default();
        let transport = HttpTransport::new(config).unwrap();

        let endpoint = A2aEndpoint {
            url: "ws://localhost:8080".to_string(),
            agent_id: "test-agent".to_string(),
            public_key: None,
        };

        let result = transport.connect(&endpoint).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("HTTP URL must start with")
        );
    }

    #[tokio::test]
    async fn test_tls_requirement() {
        let config = TransportConfig {
            tls_config: crate::transport::TlsConfig {
                require_tls: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let transport = HttpTransport::new(config).unwrap();

        let endpoint = A2aEndpoint {
            url: "http://localhost:8080".to_string(),
            agent_id: "test-agent".to_string(),
            public_key: None,
        };

        let result = transport.connect(&endpoint).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("TLS is required"));
    }

    #[tokio::test]
    async fn test_not_connected_error() {
        let config = TransportConfig::default();
        let transport = HttpTransport::new(config).unwrap();

        let request = A2aRequest::new("test_method", serde_json::json!({}));
        let result = transport.send_request(request).await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Transport not connected")
        );
    }
}
