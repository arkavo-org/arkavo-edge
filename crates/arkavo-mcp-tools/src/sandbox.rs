//! Broader tool sandbox for MCP tool execution
//!
//! Triggered by `Obligation::Sandbox` from the Task Policy Manager.
//! Uses platform-specific isolation: Docker when available, OS-level fallback.

use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

/// Sandbox execution environment for MCP tools.
pub struct ToolSandbox {
    config: ToolSandboxConfig,
}

/// Configuration for sandboxed tool execution.
#[derive(Debug, Clone)]
pub struct ToolSandboxConfig {
    pub network_access: bool,
    pub filesystem_readonly: bool,
    pub timeout_secs: u64,
    pub memory_limit_mb: u64,
    pub allowed_paths: Vec<String>,
    pub environment_vars: HashMap<String, String>,
}

impl Default for ToolSandboxConfig {
    fn default() -> Self {
        Self {
            network_access: false,
            filesystem_readonly: true,
            timeout_secs: 60,
            memory_limit_mb: 512,
            allowed_paths: vec!["/tmp".to_string()],
            environment_vars: HashMap::new(),
        }
    }
}

/// Result of sandboxed execution.
#[derive(Debug)]
pub struct SandboxResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Strategy used for sandboxing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxStrategy {
    Docker,
    OsPlatform,
    None,
}

impl ToolSandbox {
    pub fn new(config: ToolSandboxConfig) -> Self {
        Self { config }
    }

    /// Detect the best available sandboxing strategy.
    pub fn detect_strategy() -> SandboxStrategy {
        if Self::docker_available() {
            return SandboxStrategy::Docker;
        }
        if Self::os_sandbox_available() {
            return SandboxStrategy::OsPlatform;
        }
        SandboxStrategy::None
    }

    /// Execute a command in the sandbox.
    pub async fn execute(
        &self,
        command: &str,
        working_dir: Option<&str>,
    ) -> Result<SandboxResult, SandboxError> {
        let strategy = Self::detect_strategy();
        match strategy {
            SandboxStrategy::Docker => self.execute_docker(command, working_dir).await,
            SandboxStrategy::OsPlatform => self.execute_os_sandbox(command, working_dir).await,
            SandboxStrategy::None => Err(SandboxError::NoSandboxAvailable),
        }
    }

    fn docker_available() -> bool {
        std::process::Command::new("docker")
            .arg("info")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(target_os = "macos")]
    fn os_sandbox_available() -> bool {
        std::process::Command::new("sandbox-exec")
            .arg("-n")
            .arg("no-network")
            .arg("true")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(target_os = "linux")]
    fn os_sandbox_available() -> bool {
        std::process::Command::new("firejail")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn os_sandbox_available() -> bool {
        false
    }

    async fn execute_docker(
        &self,
        command: &str,
        working_dir: Option<&str>,
    ) -> Result<SandboxResult, SandboxError> {
        let mut args = vec![
            "run".to_string(),
            "--rm".to_string(),
            format!("--memory={}m", self.config.memory_limit_mb),
            "--cpus=1.0".to_string(),
        ];

        if !self.config.network_access {
            args.push("--network=none".to_string());
        }

        if self.config.filesystem_readonly {
            args.push("--read-only".to_string());
            args.push("--tmpfs=/tmp".to_string());
        }

        for path in &self.config.allowed_paths {
            let mode = if self.config.filesystem_readonly {
                "ro"
            } else {
                "rw"
            };
            args.push(format!("-v={path}:{path}:{mode}"));
        }

        for (key, val) in &self.config.environment_vars {
            args.push(format!("-e={key}={val}"));
        }

        if let Some(dir) = working_dir {
            args.push(format!("-w={dir}"));
        }

        args.push("alpine:latest".to_string());
        args.push("/bin/sh".to_string());
        args.push("-c".to_string());
        args.push(command.to_string());

        self.run_with_timeout("docker", &args).await
    }

    #[cfg(target_os = "macos")]
    async fn execute_os_sandbox(
        &self,
        command: &str,
        working_dir: Option<&str>,
    ) -> Result<SandboxResult, SandboxError> {
        let profile = self.macos_sandbox_profile();
        let mut args = vec![
            "-p".to_string(),
            profile,
            "/bin/sh".to_string(),
            "-c".to_string(),
        ];

        let full_cmd = if let Some(dir) = working_dir {
            format!("cd {dir} && {command}")
        } else {
            command.to_string()
        };
        args.push(full_cmd);

        self.run_with_timeout("sandbox-exec", &args).await
    }

    #[cfg(target_os = "linux")]
    async fn execute_os_sandbox(
        &self,
        command: &str,
        working_dir: Option<&str>,
    ) -> Result<SandboxResult, SandboxError> {
        let mut args = vec!["--quiet".to_string(), "--noprofile".to_string()];

        if !self.config.network_access {
            args.push("--net=none".to_string());
        }
        if self.config.filesystem_readonly {
            args.push("--read-only".to_string());
        }

        args.push("/bin/sh".to_string());
        args.push("-c".to_string());

        let full_cmd = if let Some(dir) = working_dir {
            format!("cd {dir} && {command}")
        } else {
            command.to_string()
        };
        args.push(full_cmd);

        self.run_with_timeout("firejail", &args).await
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    async fn execute_os_sandbox(
        &self,
        _command: &str,
        _working_dir: Option<&str>,
    ) -> Result<SandboxResult, SandboxError> {
        Err(SandboxError::PlatformNotSupported)
    }

    #[cfg(target_os = "macos")]
    fn macos_sandbox_profile(&self) -> String {
        let mut rules = vec!["(version 1)".to_string(), "(deny default)".to_string()];

        rules.push("(allow process-exec)".to_string());
        rules.push("(allow process-fork)".to_string());
        rules.push("(allow sysctl-read)".to_string());
        rules.push("(allow mach-lookup)".to_string());

        // Read access for system libraries
        rules.push("(allow file-read* (subpath \"/usr/lib\"))".to_string());
        rules.push("(allow file-read* (subpath \"/usr/bin\"))".to_string());
        rules.push("(allow file-read* (subpath \"/bin\"))".to_string());

        if !self.config.filesystem_readonly {
            for path in &self.config.allowed_paths {
                rules.push(format!("(allow file-write* (subpath \"{path}\"))"));
            }
        }

        for path in &self.config.allowed_paths {
            rules.push(format!("(allow file-read* (subpath \"{path}\"))"));
        }

        if self.config.network_access {
            rules.push("(allow network*)".to_string());
        }

        rules.join("\n")
    }

    async fn run_with_timeout(
        &self,
        program: &str,
        args: &[String],
    ) -> Result<SandboxResult, SandboxError> {
        let timeout = Duration::from_secs(self.config.timeout_secs);

        let child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| SandboxError::SpawnFailed(e.to_string()))?;

        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(output)) => Ok(SandboxResult {
                exit_code: output.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&output.stdout).to_string(),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                timed_out: false,
            }),
            Ok(Err(e)) => Err(SandboxError::ExecutionFailed(e.to_string())),
            Err(_) => {
                // Process will be killed on drop via kill_on_drop(true)
                Ok(SandboxResult {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: "Sandbox execution timed out".to_string(),
                    timed_out: true,
                })
            }
        }
    }
}

/// Errors during sandboxed execution.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("No sandbox runtime available (install Docker or firejail)")]
    NoSandboxAvailable,

    #[error("Platform not supported for OS-level sandboxing")]
    PlatformNotSupported,

    #[error("Failed to spawn sandbox process: {0}")]
    SpawnFailed(String),

    #[error("Sandbox execution failed: {0}")]
    ExecutionFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ToolSandboxConfig::default();
        assert!(!config.network_access);
        assert!(config.filesystem_readonly);
        assert_eq!(config.timeout_secs, 60);
        assert_eq!(config.memory_limit_mb, 512);
    }

    #[test]
    fn test_sandbox_result_fields() {
        let result = SandboxResult {
            exit_code: 0,
            stdout: "hello".to_string(),
            stderr: String::new(),
            timed_out: false,
        };
        assert_eq!(result.exit_code, 0);
        assert!(!result.timed_out);
    }

    #[test]
    fn test_sandbox_error_display() {
        let err = SandboxError::NoSandboxAvailable;
        assert!(err.to_string().contains("No sandbox"));
        let err = SandboxError::SpawnFailed("oops".to_string());
        assert!(err.to_string().contains("oops"));
    }

    #[test]
    fn test_custom_config() {
        let config = ToolSandboxConfig {
            network_access: true,
            filesystem_readonly: false,
            timeout_secs: 30,
            memory_limit_mb: 256,
            allowed_paths: vec!["/home".to_string()],
            environment_vars: HashMap::from([("KEY".to_string(), "val".to_string())]),
        };
        assert!(config.network_access);
        assert!(!config.filesystem_readonly);
        assert_eq!(config.allowed_paths.len(), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_sandbox_profile_generation() {
        let config = ToolSandboxConfig {
            network_access: false,
            filesystem_readonly: true,
            allowed_paths: vec!["/tmp".to_string()],
            ..Default::default()
        };
        let sandbox = ToolSandbox::new(config);
        let profile = sandbox.macos_sandbox_profile();
        assert!(profile.contains("(deny default)"));
        assert!(profile.contains("file-read*"));
        assert!(!profile.contains("network*"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_profile_with_network() {
        let config = ToolSandboxConfig {
            network_access: true,
            ..Default::default()
        };
        let sandbox = ToolSandbox::new(config);
        let profile = sandbox.macos_sandbox_profile();
        assert!(profile.contains("network*"));
    }

    #[test]
    fn test_detect_strategy_returns_value() {
        let strategy = ToolSandbox::detect_strategy();
        // Any strategy is valid depending on environment
        assert!(matches!(
            strategy,
            SandboxStrategy::Docker | SandboxStrategy::OsPlatform | SandboxStrategy::None
        ));
    }
}
