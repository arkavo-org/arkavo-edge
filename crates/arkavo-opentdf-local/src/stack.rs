//! OpenTDF stack lifecycle management.

use crate::config::{ContainerSpec, OpenTdfEndpoints, StackConfig};
use crate::error::{OpenTdfLocalError, Result};
use crate::health::{check_opentdf_health, check_orchestrator_health};
use crate::runtime::{ContainerRuntime, detect_runtime};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::process::Command;
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// Status of the OpenTDF stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StackStatus {
    /// All containers are running and healthy.
    Running,
    /// Some containers are running.
    Partial,
    /// No containers are running.
    Stopped,
    /// Unable to determine status.
    Unknown,
}

/// Information about a running container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub name: String,
    pub image: String,
    pub state: String,
    pub ip_address: Option<String>,
}

/// The local OpenTDF stack manager.
#[derive(Debug)]
pub struct OpenTdfStack {
    runtime: ContainerRuntime,
    config: StackConfig,
}

impl OpenTdfStack {
    /// Create a new stack manager with auto-detected runtime.
    pub fn new() -> Result<Self> {
        let runtime = detect_runtime()?;
        Ok(Self {
            runtime,
            config: StackConfig::default(),
        })
    }

    /// Create a stack manager with a specific runtime.
    pub fn with_runtime(runtime: ContainerRuntime) -> Self {
        Self {
            runtime,
            config: StackConfig::default(),
        }
    }

    /// Detect if an OpenTDF stack is already running.
    pub async fn detect() -> Option<Self> {
        let stack = Self::new().ok()?;

        match stack.status().await {
            Ok(StackStatus::Running | StackStatus::Partial) => Some(stack),
            _ => None,
        }
    }

    /// Get the current stack status.
    pub async fn status(&self) -> Result<StackStatus> {
        let running = self.list_running_containers().await?;
        let expected: Vec<&str> = self
            .config
            .containers
            .iter()
            .map(|c| c.name.as_str())
            .collect();

        let running_names: Vec<&str> = running.iter().map(|c| c.name.as_str()).collect();
        let running_count = expected
            .iter()
            .filter(|n| running_names.contains(n))
            .count();

        Ok(match running_count {
            0 => StackStatus::Stopped,
            n if n == expected.len() => StackStatus::Running,
            _ => StackStatus::Partial,
        })
    }

    /// Start the OpenTDF stack.
    pub async fn start(&self) -> Result<()> {
        info!("Starting local OpenTDF stack with {}", self.runtime);

        // Ensure config directories exist
        self.ensure_config_dirs().await?;

        // Create network if needed
        self.ensure_network().await?;

        // Start containers in dependency order
        for container in &self.config.containers {
            // Wait for dependencies
            for dep in &container.depends_on {
                self.wait_for_container(dep).await?;
            }

            // Start the container if not already running
            if !self.is_container_running(&container.name).await? {
                self.start_container(container).await?;
            } else {
                info!("Container {} already running", container.name);
            }
        }

        // Wait for services to be healthy
        info!("Waiting for services to become healthy...");
        self.wait_for_healthy().await?;

        info!("OpenTDF stack is ready");
        Ok(())
    }

    /// Stop the OpenTDF stack.
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping local OpenTDF stack");

        // Stop in reverse order
        for container in self.config.containers.iter().rev() {
            if self.is_container_running(&container.name).await? {
                self.stop_container(&container.name).await?;
            }
        }

        info!("OpenTDF stack stopped");
        Ok(())
    }

    /// Get the endpoints for the running stack.
    ///
    /// Note: Auth is provided by the orchestrator running on localhost:3000,
    /// not by Keycloak in a container.
    #[allow(clippy::unused_async)]
    pub async fn get_endpoints(&self) -> Result<OpenTdfEndpoints> {
        Ok(OpenTdfEndpoints {
            kas_url: "http://localhost:8080".to_string(),
            oauth_url: "http://localhost:3000".to_string(),
            token_endpoint: "http://localhost:3000/token".to_string(),
            client_id: "opentdf-sdk".to_string(),
            client_secret: "secret".to_string(),
        })
    }

    /// List running OpenTDF containers.
    async fn list_running_containers(&self) -> Result<Vec<ContainerInfo>> {
        let cmd = self.runtime.command();

        // Apple container CLI uses 'list' instead of 'ps'
        let list_cmd = if self.runtime == ContainerRuntime::AppleContainer {
            "list"
        } else {
            "ps"
        };

        let output = if self.runtime == ContainerRuntime::AppleContainer {
            // Apple container CLI output format is different
            Command::new(cmd)
                .arg(list_cmd)
                .output()
                .await
                .map_err(|e| OpenTdfLocalError::RuntimeError(e.to_string()))?
        } else {
            Command::new(cmd)
                .args([list_cmd, "--format", "{{.Names}}\t{{.Image}}\t{{.State}}"])
                .output()
                .await
                .map_err(|e| OpenTdfLocalError::RuntimeError(e.to_string()))?
        };

        if !output.status.success() {
            return Ok(vec![]);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        let containers: Vec<ContainerInfo> = if self.runtime == ContainerRuntime::AppleContainer {
            // Parse Apple container list format:
            // ID          IMAGE                                 OS     ARCH   STATE    ADDR          CPUS  MEMORY
            // opentdf     registry.opentdf.io/platform:nightly  linux  arm64  running  192.168.65.8  4     1024 MB
            stdout
                .lines()
                .skip(1) // Skip header line
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 5 {
                        let name = parts[0].to_string();
                        // Only include OpenTDF-related containers (no longer includes keycloak)
                        if ["opentdf", "opentdfdb"].iter().any(|n| name == *n) {
                            let ip = if parts.len() >= 6 {
                                Some(parts[5].to_string())
                            } else {
                                None
                            };
                            return Some(ContainerInfo {
                                name,
                                image: parts[1].to_string(),
                                state: parts[4].to_string(),
                                ip_address: ip,
                            });
                        }
                    }
                    None
                })
                .collect()
        } else {
            // Docker/Podman format
            stdout
                .lines()
                .filter_map(|line| {
                    let parts: Vec<&str> = line.split('\t').collect();
                    if parts.len() >= 3 {
                        let name = parts[0].to_string();
                        // Only include OpenTDF-related containers (no longer includes keycloak)
                        if ["opentdf", "opentdfdb"].iter().any(|n| name.contains(n)) {
                            return Some(ContainerInfo {
                                name,
                                image: parts[1].to_string(),
                                state: parts[2].to_string(),
                                ip_address: None,
                            });
                        }
                    }
                    None
                })
                .collect()
        };

        Ok(containers)
    }

    /// Check if a specific container is running.
    async fn is_container_running(&self, name: &str) -> Result<bool> {
        let cmd = self.runtime.command();

        let output = Command::new(cmd)
            .args(["ps", "-q", "--filter", &format!("name={name}")])
            .output()
            .await
            .map_err(|e| OpenTdfLocalError::RuntimeError(e.to_string()))?;

        Ok(!String::from_utf8_lossy(&output.stdout).trim().is_empty())
    }

    /// Ensure the network exists.
    async fn ensure_network(&self) -> Result<()> {
        let cmd = self.runtime.command();
        let network = &self.config.network;

        // Check if network exists
        let check = Command::new(cmd)
            .args([
                "network",
                "ls",
                "-q",
                "--filter",
                &format!("name={network}"),
            ])
            .output()
            .await
            .map_err(|e| OpenTdfLocalError::RuntimeError(e.to_string()))?;

        if !String::from_utf8_lossy(&check.stdout).trim().is_empty() {
            debug!("Network {network} already exists");
            return Ok(());
        }

        // Create network
        info!("Creating network: {network}");
        let output = Command::new(cmd)
            .args(["network", "create", network])
            .output()
            .await
            .map_err(|e| OpenTdfLocalError::RuntimeError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore if network already exists
            if !stderr.contains("already exists") {
                return Err(OpenTdfLocalError::NetworkError(stderr.to_string()));
            }
        }

        Ok(())
    }

    /// Start a single container.
    async fn start_container(&self, spec: &ContainerSpec) -> Result<()> {
        let cmd = self.runtime.command();
        info!("Starting container: {}", spec.name);

        let mut args = vec![
            "run".to_string(),
            "-d".to_string(),
            "--name".to_string(),
            spec.name.clone(),
            "--network".to_string(),
            self.config.network.clone(),
        ];

        // Add port mappings
        for (host, container) in &spec.ports {
            args.push("-p".to_string());
            args.push(format!("{host}:{container}"));
        }

        // Add environment variables
        for (key, value) in &spec.env {
            args.push("-e".to_string());
            args.push(format!("{key}={value}"));
        }

        // Add volume mounts
        for (host, container) in &spec.mounts {
            args.push("-v".to_string());
            args.push(format!("{host}:{container}"));
        }

        // Add image
        args.push(spec.image.clone());

        // Add command arguments
        args.extend(spec.args.clone());

        debug!("Running: {} {}", cmd, args.join(" "));

        let output = Command::new(cmd).args(&args).output().await.map_err(|e| {
            OpenTdfLocalError::ContainerStartFailed {
                name: spec.name.clone(),
                reason: e.to_string(),
            }
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(OpenTdfLocalError::ContainerStartFailed {
                name: spec.name.clone(),
                reason: stderr.to_string(),
            });
        }

        Ok(())
    }

    /// Stop a single container.
    async fn stop_container(&self, name: &str) -> Result<()> {
        let cmd = self.runtime.command();
        info!("Stopping container: {name}");

        let output = Command::new(cmd)
            .args(["stop", name])
            .output()
            .await
            .map_err(|e| OpenTdfLocalError::ContainerStopFailed {
                name: name.to_string(),
                reason: e.to_string(),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Failed to stop {name}: {stderr}");
        }

        // Remove the container
        let _ = Command::new(cmd).args(["rm", "-f", name]).output().await;

        Ok(())
    }

    /// Wait for a container to be running.
    async fn wait_for_container(&self, name: &str) -> Result<()> {
        for _ in 0..30 {
            if self.is_container_running(name).await? {
                return Ok(());
            }
            sleep(Duration::from_secs(1)).await;
        }

        Err(OpenTdfLocalError::Timeout {
            service: name.to_string(),
        })
    }

    /// Wait for all services to be healthy.
    async fn wait_for_healthy(&self) -> Result<()> {
        // Wait for orchestrator OIDC (must be started separately)
        for attempt in 1..=30 {
            debug!("Waiting for orchestrator OIDC (attempt {attempt}/30)");
            if check_orchestrator_health("http://localhost:3000")
                .await
                .unwrap_or(false)
            {
                break;
            }
            if attempt == 30 {
                warn!(
                    "Orchestrator OIDC not available - start orchestrator with: arkavo orchestrator start"
                );
            }
            sleep(Duration::from_secs(1)).await;
        }

        // Wait for OpenTDF
        for attempt in 1..=30 {
            debug!("Waiting for OpenTDF (attempt {attempt}/30)");
            if check_opentdf_health("http://localhost:8080")
                .await
                .unwrap_or(false)
            {
                break;
            }
            if attempt == 30 {
                return Err(OpenTdfLocalError::Timeout {
                    service: "OpenTDF".to_string(),
                });
            }
            sleep(Duration::from_secs(2)).await;
        }

        Ok(())
    }

    /// Ensure configuration directories exist and generate config files.
    async fn ensure_config_dirs(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.config.config_dir)
            .await
            .map_err(|e| {
                OpenTdfLocalError::ConfigError(format!("Failed to create config dir: {e}"))
            })?;

        tokio::fs::create_dir_all(&self.config.keys_dir)
            .await
            .map_err(|e| {
                OpenTdfLocalError::ConfigError(format!("Failed to create keys dir: {e}"))
            })?;

        // Generate opentdf.yaml config pointing to orchestrator OIDC
        self.write_opentdf_config().await?;

        Ok(())
    }

    /// Write the OpenTDF platform configuration file.
    ///
    /// This config points the OpenTDF platform to the orchestrator's built-in OIDC
    /// provider instead of Keycloak.
    async fn write_opentdf_config(&self) -> Result<()> {
        let config_path = format!("{}/opentdf.yaml", self.config.config_dir);

        // host.containers.internal resolves to the host machine from Apple containers
        // For Docker/Podman on Linux, use host.docker.internal or gateway IP
        let host_internal = if self.runtime == ContainerRuntime::AppleContainer {
            "host.containers.internal"
        } else {
            "host.docker.internal"
        };

        let config = format!(
            r#"# OpenTDF Platform Configuration
# Auto-generated by arkavo - uses orchestrator OIDC provider

server:
  auth:
    enabled: true
    issuer: http://{host_internal}:3000
    audience: opentdf
    policy:
      default: allow
  grpc:
    reflection: true
  http:
    enabled: true

logger:
  level: info
  type: text

db:
  host: opentdfdb
  port: 5432
  user: postgres
  password: changeme
  database: opentdf
  sslmode: disable

services:
  kas:
    enabled: true
  policy:
    enabled: true
  authorization:
    enabled: true
  entityresolution:
    enabled: true
    url: http://{host_internal}:3000/entityresolution
  wellknown:
    enabled: true
"#
        );

        tokio::fs::write(&config_path, config).await.map_err(|e| {
            OpenTdfLocalError::ConfigError(format!("Failed to write opentdf.yaml: {e}"))
        })?;

        debug!("Wrote OpenTDF config to {config_path}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_stack_creation() {
        // This will fail if no runtime is available, which is expected in CI
        let result = OpenTdfStack::new();
        // Just verify it doesn't panic
        let _ = result;
    }
}
