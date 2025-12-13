//! Container runtime detection for local OpenTDF stack.

use crate::error::{OpenTdfLocalError, Result};
use std::process::Command;
use tracing::{debug, info};

/// Supported container runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerRuntime {
    /// Docker
    Docker,
    /// Podman
    Podman,
}

impl ContainerRuntime {
    /// Get the command name for this runtime.
    #[must_use]
    pub fn command(&self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }
}

impl std::fmt::Display for ContainerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Docker => write!(f, "Docker"),
            Self::Podman => write!(f, "Podman"),
        }
    }
}

/// Detect the best available container runtime.
///
/// Prefers Docker over Podman.
pub fn detect_runtime() -> Result<ContainerRuntime> {
    // Prefer Docker
    if check_runtime("docker") {
        info!("Detected Docker");
        return Ok(ContainerRuntime::Docker);
    }

    // Fall back to Podman
    if check_runtime("podman") {
        info!("Detected Podman");
        return Ok(ContainerRuntime::Podman);
    }

    Err(OpenTdfLocalError::NoRuntimeFound)
}

/// Check if a runtime command is available.
fn check_runtime(cmd: &str) -> bool {
    let result = Command::new(cmd).arg("--version").output();

    match result {
        Ok(output) => {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout);
                debug!("{cmd} version: {}", version.trim());
                true
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_command_names() {
        assert_eq!(ContainerRuntime::Docker.command(), "docker");
        assert_eq!(ContainerRuntime::Podman.command(), "podman");
    }
}
