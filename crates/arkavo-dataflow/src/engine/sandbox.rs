use anyhow::Result;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SandboxConfig {
    pub enable_network: bool,
    pub memory_limit_mb: Option<u64>,
    pub cpu_limit_percent: Option<u32>,
    pub allowed_paths: Vec<String>,
    pub environment_vars: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            enable_network: false,
            memory_limit_mb: Some(512),
            cpu_limit_percent: Some(50),
            allowed_paths: vec!["/tmp".to_string()],
            environment_vars: vec![],
        }
    }
}

pub struct Sandbox {
    config: Arc<SandboxConfig>,
    #[allow(dead_code)]
    sandbox_id: String,
}

impl Sandbox {
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config: Arc::new(config),
            sandbox_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    #[cfg(target_os = "linux")]
    pub fn execute_transform(
        &self,
        _code: &str,
        _input: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Linux implementation using seccomp
        self.execute_linux_sandbox(_code, _input)
    }

    #[cfg(target_os = "macos")]
    pub fn execute_transform(
        &self,
        code: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // macOS implementation using sandbox-exec
        self.execute_macos_sandbox(code, input)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub fn execute_transform(
        &self,
        _code: &str,
        _input: serde_json::Value,
    ) -> Result<serde_json::Value> {
        Err(anyhow::anyhow!("Sandbox not supported on this platform"))
    }

    #[cfg(target_os = "linux")]
    fn execute_linux_sandbox(
        &self,
        code: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Create a temporary file for the transform code
        let temp_dir = tempfile::tempdir()?;
        let script_path = temp_dir.path().join("transform.js");
        std::fs::write(&script_path, code)?;

        let input_json = serde_json::to_string(&input)?;

        // Build seccomp command
        let mut cmd = Command::new("firejail");

        // Add security options
        cmd.arg("--noprofile").arg("--quiet");

        if !self.config.enable_network {
            cmd.arg("--net=none");
        }

        if let Some(memory_mb) = self.config.memory_limit_mb {
            cmd.arg(format!("--rlimit-as={}m", memory_mb));
        }

        // Add allowed paths
        for path in &self.config.allowed_paths {
            cmd.arg(format!("--whitelist={}", path));
        }

        // Execute node.js with the transform
        cmd.arg("--").arg("node").arg("-e").arg(format!(
            r#"
                const input = {};
                const transform = require('{}');
                const result = transform(input);
                console.log(JSON.stringify(result));
                "#,
            input_json,
            script_path.display()
        ));

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Sandbox execution failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: serde_json::Value = serde_json::from_str(stdout.trim())?;

        Ok(result)
    }

    #[cfg(target_os = "macos")]
    fn execute_macos_sandbox(
        &self,
        code: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value> {
        // Create a temporary file for the transform code
        let temp_dir = tempfile::tempdir()?;
        let script_path = temp_dir.path().join("transform.js");
        std::fs::write(&script_path, code)?;

        let input_json = serde_json::to_string(&input)?;

        // Create sandbox profile
        let sandbox_profile = self.generate_macos_sandbox_profile();
        let profile_path = temp_dir.path().join("sandbox.sb");
        std::fs::write(&profile_path, sandbox_profile)?;

        // Execute with sandbox-exec
        let mut cmd = Command::new("sandbox-exec");
        cmd.arg("-f")
            .arg(profile_path)
            .arg("node")
            .arg("-e")
            .arg(format!(
                r#"
                const input = {};
                const transform = require('{}');
                const result = transform(input);
                console.log(JSON.stringify(result));
                "#,
                input_json,
                script_path.display()
            ));

        let output = cmd.output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Sandbox execution failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result: serde_json::Value = serde_json::from_str(stdout.trim())?;

        Ok(result)
    }

    #[cfg(target_os = "macos")]
    fn generate_macos_sandbox_profile(&self) -> String {
        let mut profile = String::from(
            r#"(version 1)
(deny default)
(allow process-exec (literal "/usr/local/bin/node"))
(allow file-read*)
(allow file-write-data (regex #"^/private/tmp/.*"))
(allow mach-lookup)
"#,
        );

        if !self.config.enable_network {
            profile.push_str("(deny network*)\n");
        }

        for path in &self.config.allowed_paths {
            use std::fmt::Write;
            writeln!(profile, "(allow file-write* (regex #\"^{path}.*\"))").unwrap();
        }

        profile
    }

    pub fn validate_transform_code(code: &str) -> Result<()> {
        // Basic validation to prevent obvious malicious code
        let forbidden_patterns = [
            "require('child_process')",
            "require('fs')",
            "require('net')",
            "require('http')",
            "require('https')",
            "eval(",
            "Function(",
            "setTimeout(",
            "setInterval(",
            "__proto__",
            "constructor",
        ];

        for pattern in &forbidden_patterns {
            if code.contains(pattern) {
                return Err(anyhow::anyhow!("Forbidden pattern detected: {}", pattern));
            }
        }

        Ok(())
    }
}

pub struct SandboxPool {
    sandboxes: Arc<RwLock<Vec<Sandbox>>>,
    config: SandboxConfig,
    max_sandboxes: usize,
}

impl SandboxPool {
    pub fn new(config: SandboxConfig, max_sandboxes: usize) -> Self {
        Self {
            sandboxes: Arc::new(RwLock::new(Vec::with_capacity(max_sandboxes))),
            config,
            max_sandboxes,
        }
    }

    pub async fn get_sandbox(&self) -> Result<Sandbox> {
        let mut sandboxes = self.sandboxes.write().await;

        if let Some(sandbox) = sandboxes.pop() {
            Ok(sandbox)
        } else if sandboxes.len() < self.max_sandboxes {
            Ok(Sandbox::new(self.config.clone()))
        } else {
            Err(anyhow::anyhow!("No available sandboxes"))
        }
    }

    pub async fn return_sandbox(&self, sandbox: Sandbox) {
        let mut sandboxes = self.sandboxes.write().await;
        if sandboxes.len() < self.max_sandboxes {
            sandboxes.push(sandbox);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_transform_code() {
        // Valid code should pass
        assert!(
            Sandbox::validate_transform_code("function transform(input) { return input; }").is_ok()
        );

        // Code with forbidden patterns should fail
        assert!(
            Sandbox::validate_transform_code("require('fs').readFileSync('/etc/passwd')").is_err()
        );
        assert!(Sandbox::validate_transform_code("eval('malicious code')").is_err());
        assert!(
            Sandbox::validate_transform_code("require('child_process').exec('rm -rf /')").is_err()
        );
    }

    #[test]
    fn test_sandbox_config_default() {
        let config = SandboxConfig::default();
        assert!(!config.enable_network);
        assert_eq!(config.memory_limit_mb, Some(512));
        assert_eq!(config.cpu_limit_percent, Some(50));
        assert_eq!(config.allowed_paths, vec!["/tmp".to_string()]);
    }
}
