//! Container runtime detection for local OpenTDF stack.

use crate::error::{OpenTdfLocalError, Result};
use std::process::Command;
use tracing::{debug, info};

/// Supported container runtimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerRuntime {
    /// Apple macOS native container CLI
    AppleContainer,
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
            Self::AppleContainer => "container",
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }

    /// Whether this runtime uses IP addresses instead of hostnames.
    /// Apple container CLI doesn't resolve container hostnames on the host.
    #[must_use]
    pub fn requires_ip_addresses(&self) -> bool {
        matches!(self, Self::AppleContainer)
    }

    /// Whether this runtime only supports directory mounts (not file mounts).
    #[must_use]
    pub fn directory_mounts_only(&self) -> bool {
        matches!(self, Self::AppleContainer)
    }
}

impl std::fmt::Display for ContainerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AppleContainer => write!(f, "Apple Container"),
            Self::Docker => write!(f, "Docker"),
            Self::Podman => write!(f, "Podman"),
        }
    }
}

/// Detect the best available container runtime.
///
/// On macOS, prefers Apple container CLI over Docker/Podman.
/// On other platforms, falls back to Docker or Podman.
pub fn detect_runtime() -> Result<ContainerRuntime> {
    // On macOS, prefer Apple container CLI
    #[cfg(target_os = "macos")]
    {
        if check_runtime("container") {
            info!("Detected Apple container CLI");
            return Ok(ContainerRuntime::AppleContainer);
        }
    }

    // Fall back to Docker
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
        assert_eq!(ContainerRuntime::AppleContainer.command(), "container");
        assert_eq!(ContainerRuntime::Docker.command(), "docker");
        assert_eq!(ContainerRuntime::Podman.command(), "podman");
    }

    #[test]
    fn apple_container_requires_ip() {
        assert!(ContainerRuntime::AppleContainer.requires_ip_addresses());
        assert!(!ContainerRuntime::Docker.requires_ip_addresses());
    }
}
