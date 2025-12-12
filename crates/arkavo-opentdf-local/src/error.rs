//! Error types for local OpenTDF stack management.

use thiserror::Error;

/// Errors that can occur during local OpenTDF operations.
#[derive(Error, Debug)]
pub enum OpenTdfLocalError {
    #[error("No container runtime found (tried: container, docker, podman)")]
    NoRuntimeFound,

    #[error("Container runtime error: {0}")]
    RuntimeError(String),

    #[error("Failed to start container {name}: {reason}")]
    ContainerStartFailed { name: String, reason: String },

    #[error("Failed to stop container {name}: {reason}")]
    ContainerStopFailed { name: String, reason: String },

    #[error("Container {name} is not running")]
    ContainerNotRunning { name: String },

    #[error("Health check failed for {service}: {reason}")]
    HealthCheckFailed { service: String, reason: String },

    #[error("Network creation failed: {0}")]
    NetworkError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Timeout waiting for {service} to become healthy")]
    Timeout { service: String },
}

/// Result type for local OpenTDF operations.
pub type Result<T> = std::result::Result<T, OpenTdfLocalError>;
