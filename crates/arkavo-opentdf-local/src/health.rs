//! Health checks for local OpenTDF stack components.

use crate::error::Result;
use tracing::{debug, warn};

/// Check if the OpenTDF platform is healthy.
pub(crate) async fn check_opentdf_health(base_url: &str) -> Result<bool> {
    let url = format!("{base_url}/.well-known/opentdf-configuration");
    check_http_endpoint(&url, "OpenTDF").await
}

/// Check if Keycloak is healthy (legacy, for backwards compatibility).
#[allow(dead_code)]
pub(crate) async fn check_keycloak_health(base_url: &str) -> Result<bool> {
    let url = format!("{base_url}/realms/opentdf");
    check_http_endpoint(&url, "Keycloak").await
}

/// Check if the orchestrator OIDC endpoint is healthy.
pub(crate) async fn check_orchestrator_health(base_url: &str) -> Result<bool> {
    let url = format!("{base_url}/.well-known/openid-configuration");
    check_http_endpoint(&url, "Orchestrator OIDC").await
}

/// Check an HTTP endpoint for health.
async fn check_http_endpoint(url: &str, service: &str) -> Result<bool> {
    // Use a simple TCP connection check since we don't have reqwest here
    // This will be called after containers are started
    let parsed = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url);

    let host_port = parsed.split('/').next().unwrap_or(parsed);
    let parts: Vec<&str> = host_port.split(':').collect();

    let (host, port) = if parts.len() == 2 {
        (
            parts[0],
            parts[1].parse::<u16>().unwrap_or(match url.starts_with("https") {
                true => 443,
                false => 80,
            }),
        )
    } else {
        (
            parts[0],
            match url.starts_with("https") {
                true => 443,
                false => 80,
            },
        )
    };

    let addr = format!("{host}:{port}");

    match tokio::net::TcpStream::connect(&addr).await {
        Ok(_) => {
            debug!("{service} at {url} is reachable");
            Ok(true)
        }
        Err(e) => {
            warn!("{service} at {url} not reachable: {e}");
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_check_endpoint_invalid() {
        let result = check_http_endpoint("http://127.0.0.1:59999", "test").await;
        // Should return Ok(false) for unreachable endpoint, not error
        assert!(matches!(result, Ok(false)));
    }
}
