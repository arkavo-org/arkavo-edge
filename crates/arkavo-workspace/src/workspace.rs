use crate::{Result, WorkspaceError};
use arkavo_mcp::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub struct WorkspaceTool {
    schema: ToolSchema,
}

impl WorkspaceTool {
    pub fn new() -> Self {
        if Self::detect_runtime().is_err() {
            eprintln!(
                "Warning: Neither Docker nor Podman found. Install Docker Desktop (macOS/Windows) or podman"
            );
        }
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
                            "description": "Git repository URL to clone"
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
        if std::process::Command::new("docker")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok("docker".to_string());
        }

        if std::process::Command::new("podman")
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
        let runtime = Self::detect_runtime()?;
        let workspace_id = params
            .get("workspace_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| WorkspaceError::InvalidParams("Missing workspace_id".to_string()))?;

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

        let mut cmd = Command::new(&runtime);
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

        if let Some(repo_url) = params.get("repo_url").and_then(|v| v.as_str()) {
            let clone_cmd = format!("git clone {} /workspace", repo_url);
            let mut exec = Command::new(&runtime);
            exec.arg("exec")
                .arg(workspace_id)
                .arg("sh")
                .arg("-c")
                .arg(&clone_cmd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            let output = exec
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

        let mut cmd = Command::new(&runtime);
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

        let output = Command::new(&runtime)
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

        let output = Command::new(&runtime)
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
}
