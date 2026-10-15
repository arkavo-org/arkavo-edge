use arkavo_observability::{ObservabilityConfig, init_observability};
use arkavo_test_macros::spec;

/// Exercises the full init path in its own process: a custom log level is
/// applied through the EnvFilter, the OTLP-enabled branch is taken, and the
/// global subscriber is set exactly once. With no collector reachable the
/// init still succeeds and continues with console logging.
///
/// Single test on purpose: the global subscriber is process-wide, so parallel
/// tests initializing it would race.
#[spec("OBS-001", "OBS-007")]
#[test]
fn test_init_observability_with_otlp_enabled() {
    let config = ObservabilityConfig {
        service_name: "arkavo-edge-test".to_string(),
        console_logging: true,
        log_level: "debug".to_string(),
        enable_otlp: true,
        otlp_endpoint: Some("http://localhost:4317".to_string()),
        ..Default::default()
    };

    let handle = init_observability(config).expect("init_observability must succeed");

    tracing::info!("observability initialized for spec test");
    tracing::debug!("debug level visible through configured EnvFilter");

    // The global subscriber can only be installed once per process; a second
    // initialization must report an error rather than replacing it.
    assert!(init_observability(ObservabilityConfig::default()).is_err());

    handle.shutdown();
}
