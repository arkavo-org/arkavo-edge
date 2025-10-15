use std::time::Duration;
use tracing::{debug, info};

/// Agent-driven environment detection for observability configuration
/// Following the zero-config principle, the agent autonomously detects
/// and configures observability based on the runtime environment
pub struct ObservabilityDetector;

impl ObservabilityDetector {
    /// Detect if OTLP collector is available in the environment
    /// This is called by the agent to determine if external telemetry should be enabled
    pub async fn detect_otlp_collector() -> Option<String> {
        // Common OTLP endpoints the agent checks
        let common_endpoints = [
            "http://localhost:4317",      // Default OTLP gRPC
            "http://localhost:4318",      // Default OTLP HTTP
            "http://otel-collector:4317", // K8s service name
            "http://jaeger:4317",         // Jaeger OTLP
        ];

        for endpoint in &common_endpoints {
            if Self::check_endpoint_health(endpoint).await {
                info!(
                    endpoint = %endpoint,
                    "Agent detected available OTLP collector"
                );
                return Some(endpoint.to_string());
            }
        }

        debug!("Agent did not detect any OTLP collectors - metrics will be AG-UI only");
        None
    }

    /// Check if an OTLP endpoint is healthy
    async fn check_endpoint_health(endpoint: &str) -> bool {
        // Simple TCP connect check to see if port is open
        // In production, this could do a proper health check
        let addr = endpoint.replace("http://", "").replace("https://", "");

        match tokio::time::timeout(
            Duration::from_secs(1),
            tokio::net::TcpStream::connect(&addr),
        )
        .await
        {
            Ok(Ok(_)) => {
                debug!(endpoint = %endpoint, "OTLP endpoint is reachable");
                true
            }
            _ => false,
        }
    }

    /// Detect if running in development/staging environment
    /// Agent uses this to determine if verbose telemetry should be enabled
    pub fn detect_development_environment() -> bool {
        // Agent checks various signals
        let signals = [
            // Check if running from cargo
            std::env::var("CARGO").is_ok(),
            // Check if in debug build
            cfg!(debug_assertions),
            // Check for development indicators
            std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(|s| s.contains("target/debug")))
                .unwrap_or(false),
        ];

        let is_dev = signals.iter().any(|&s| s);

        if is_dev {
            info!("Agent detected development environment - enabling verbose telemetry");
        }

        is_dev
    }

    /// Agent determines appropriate log level based on environment
    pub fn determine_log_level() -> String {
        if Self::detect_development_environment() {
            "debug".to_string()
        } else {
            "info".to_string()
        }
    }

    /// Agent configures observability based on detected environment
    pub async fn auto_configure() -> super::ObservabilityConfig {
        let mut config = super::ObservabilityConfig {
            log_level: Self::determine_log_level(),
            ..Default::default()
        };

        // Agent checks for OTLP collector
        if let Some(endpoint) = Self::detect_otlp_collector().await {
            config.otlp_endpoint = Some(endpoint);
            config.enable_otlp = true;
            info!("Agent enabled OTLP export based on environment detection");
        }

        config
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_development_detection() {
        // This test will behave differently in dev vs release builds
        let is_dev = ObservabilityDetector::detect_development_environment();

        #[cfg(debug_assertions)]
        assert!(is_dev, "Should detect development in debug builds");

        #[cfg(not(debug_assertions))]
        assert!(!is_dev, "Should not detect development in release builds");
    }

    #[test]
    fn test_log_level_determination() {
        let level = ObservabilityDetector::determine_log_level();

        #[cfg(debug_assertions)]
        assert_eq!(level, "debug");

        #[cfg(not(debug_assertions))]
        assert_eq!(level, "info");
    }

    #[tokio::test]
    async fn test_otlp_detection() {
        // This test will only find OTLP if one is actually running
        let endpoint = ObservabilityDetector::detect_otlp_collector().await;

        // We can't assert presence/absence since it depends on environment
        // Just verify the function completes without panicking
        if let Some(ep) = endpoint {
            assert!(ep.starts_with("http://"));
        }
    }
}
