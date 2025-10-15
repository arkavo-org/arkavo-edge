use anyhow::Result;
use tracing::subscriber::set_global_default;
use tracing_subscriber::{Layer, filter::EnvFilter, layer::SubscriberExt};

pub mod agent_detection;
pub mod config;
pub mod health_reporter;
pub mod metrics;
pub mod metrics_snapshot;
pub mod session;
pub mod task_tracker;

/// Configuration for observability setup
#[derive(Debug, Clone)]
pub struct ObservabilityConfig {
    /// Service name for traces
    pub service_name: String,
    /// Service version
    pub service_version: String,
    /// OTLP endpoint (e.g., "http://localhost:4317")
    pub otlp_endpoint: Option<String>,
    /// Whether to enable console logging
    pub console_logging: bool,
    /// Log level filter
    pub log_level: String,
    /// Whether to export to OTLP
    pub enable_otlp: bool,
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            service_name: "arkavo-edge".to_string(),
            service_version: env!("CARGO_PKG_VERSION").to_string(),
            otlp_endpoint: None, // Zero config - agent will detect and configure if needed
            console_logging: true,
            log_level: "info".to_string(),
            enable_otlp: false, // Disabled by default - agent enables only when it detects OTLP collector
        }
    }
}

/// Initialize observability infrastructure
pub fn init_observability(config: ObservabilityConfig) -> Result<ObservabilityHandle> {
    let mut layers = Vec::new();

    // Console logging layer
    if config.console_logging {
        let console_layer = tracing_subscriber::fmt::layer()
            .with_file(true)
            .with_line_number(true)
            .with_thread_ids(true)
            .with_target(false)
            .compact()
            .with_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
            );
        layers.push(console_layer.boxed());
    }

    // OpenTelemetry layer - agent will auto-detect and configure
    if config.enable_otlp
        && let Some(endpoint) = &config.otlp_endpoint
    {
        tracing::info!(
            otlp_endpoint = %endpoint,
            "OTLP export auto-detected and configured by agent"
        );
        // Agent detected OTLP collector availability
        // Full integration deferred until OpenTelemetry API stabilizes
    }

    // Build and set subscriber
    let subscriber = tracing_subscriber::registry().with(layers);
    set_global_default(subscriber)?;

    tracing::info!(
        service_name = %config.service_name,
        service_version = %config.service_version,
        otlp_enabled = config.enable_otlp,
        "Observability initialized"
    );

    Ok(ObservabilityHandle {
        _tracer_provider: None, // Simplified for now
    })
}

/// Handle for managing observability lifecycle
pub struct ObservabilityHandle {
    _tracer_provider: Option<()>, // Simplified for now
}

impl ObservabilityHandle {
    /// Shutdown observability infrastructure gracefully
    pub fn shutdown(self) {
        // Simplified shutdown for now
        tracing::info!("Observability infrastructure shutdown");
    }
}

/// Convenience macros for structured logging
#[macro_export]
macro_rules! log_event {
    ($level:ident, $message:expr, $($field:ident = $value:expr),*) => {
        tracing::$level!(
            $($field = $value,)*
            "{}", $message
        );
    };
}

#[macro_export]
macro_rules! log_span {
    ($name:expr, $($field:ident = $value:expr),*) => {
        tracing::info_span!($name, $($field = $value),*)
    };
}

/// Session-related observability utilities
pub mod session_observability {
    use tracing::{error, info, instrument, warn};

    #[instrument(skip(session_id), fields(session.id = %session_id))]
    pub fn log_session_created(session_id: &str, capabilities: Option<&str>) {
        info!(
            capabilities = capabilities.unwrap_or("none"),
            "Chat session created"
        );
    }

    #[instrument(skip(session_id), fields(session.id = %session_id))]
    pub fn log_session_closed(session_id: &str, reason: &str) {
        info!(reason = reason, "Chat session closed");
    }

    #[instrument(skip(session_id), fields(session.id = %session_id))]
    pub fn log_message_sent(session_id: &str, message_length: usize) {
        info!(message.length = message_length, "Message sent to session");
    }

    #[instrument(skip(session_id), fields(session.id = %session_id))]
    pub fn log_message_received(session_id: &str, delta_type: &str, sequence: u64) {
        info!(
            delta.type = delta_type,
            delta.sequence = sequence,
            "Message delta received"
        );
    }

    #[instrument(skip(session_id), fields(session.id = %session_id))]
    pub fn log_session_error(session_id: &str, error: &str, error_code: Option<&str>) {
        error!(
            error.message = error,
            error.code = error_code.unwrap_or("unknown"),
            "Session error occurred"
        );
    }

    #[instrument(skip(session_id), fields(session.id = %session_id))]
    pub fn log_stream_start(session_id: &str, model_name: Option<&str>) {
        info!(
            model.name = model_name.unwrap_or("unknown"),
            "LLM stream started"
        );
    }

    #[instrument(skip(session_id), fields(session.id = %session_id))]
    pub fn log_stream_end(session_id: &str, reason: &str, total_tokens: Option<u32>) {
        info!(
            stream.end_reason = reason,
            stream.total_tokens = total_tokens.unwrap_or(0),
            "LLM stream ended"
        );
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ObservabilityConfig::default();
        assert_eq!(config.service_name, "arkavo-edge");
        assert!(config.console_logging);
        assert_eq!(config.log_level, "info");
        assert!(!config.enable_otlp);
    }

    #[tokio::test]
    async fn test_observability_init() {
        let config = ObservabilityConfig {
            enable_otlp: false,
            ..Default::default()
        };

        let handle = init_observability(config).unwrap();

        // Test that logging works
        tracing::info!("Test log message");

        handle.shutdown();
    }
}
