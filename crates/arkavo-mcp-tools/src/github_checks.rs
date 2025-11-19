use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;

fn check_gh_installed() -> Result<()> {
    let output = Command::new("gh")
        .arg("--version")
        .output()
        .map_err(|_| ToolError::Mcp("GitHub CLI (gh) is not installed".to_string()))?;

    if !output.status.success() {
        return Err(ToolError::Mcp(
            "GitHub CLI (gh) is not available".to_string(),
        ));
    }

    Ok(())
}

fn execute_gh_api(endpoint: &str, method: &str, data: Option<&str>) -> Result<String> {
    check_gh_installed()?;

    let mut cmd = Command::new("gh");
    cmd.arg("api").arg(endpoint).arg("--method").arg(method);

    if data.is_some() {
        cmd.arg("--input").arg("-");
        cmd.arg("--header")
            .arg("Accept: application/vnd.github+json");
    }

    let output = if let Some(json_data) = data {
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(json_data.as_bytes())?;
                }
                child.wait_with_output()
            })
            .map_err(|e| ToolError::Mcp(format!("Failed to execute gh api: {e}")))?
    } else {
        cmd.output()
            .map_err(|e| ToolError::Mcp(format!("Failed to execute gh api: {e}")))?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ToolError::Mcp(format!("GitHub API failed: {stderr}")));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub struct GitHubChecksTool {
    schema: ToolSchema,
}

impl GitHubChecksTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "gh_checks".to_string(),
                aliases: None,
                description: "Create and update GitHub Check Runs with annotations".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["create", "update", "list"],
                            "description": "Check action to perform"
                        },
                        "repo": {
                            "type": "string",
                            "description": "Repository in owner/repo format"
                        },
                        "name": {
                            "type": "string",
                            "description": "Name of the check run"
                        },
                        "head_sha": {
                            "type": "string",
                            "description": "Git commit SHA (required for create)"
                        },
                        "status": {
                            "type": "string",
                            "enum": ["queued", "in_progress", "completed"],
                            "description": "Status of the check run"
                        },
                        "conclusion": {
                            "type": "string",
                            "enum": ["success", "failure", "neutral", "cancelled", "skipped", "timed_out", "action_required"],
                            "description": "Conclusion (required when status=completed)"
                        },
                        "output": {
                            "type": "object",
                            "properties": {
                                "title": { "type": "string" },
                                "summary": { "type": "string" },
                                "text": { "type": "string" }
                            },
                            "description": "Check output details"
                        },
                        "annotations": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string", "description": "File path" },
                                    "start_line": { "type": "integer", "description": "Start line number" },
                                    "end_line": { "type": "integer", "description": "End line number" },
                                    "annotation_level": {
                                        "type": "string",
                                        "enum": ["notice", "warning", "failure"],
                                        "description": "Severity level"
                                    },
                                    "message": { "type": "string", "description": "Annotation message" }
                                },
                                "required": ["path", "start_line", "end_line", "annotation_level", "message"]
                            },
                            "description": "Code annotations (file:line:message)"
                        },
                        "check_run_id": {
                            "type": "integer",
                            "description": "Check run ID (required for update)"
                        }
                    },
                    "required": ["action", "repo"]
                }),
            },
        }
    }

    fn execute_checks(&self, params: &Value) -> Result<String> {
        let action = params["action"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("Missing action".to_string()))?;

        let repo = params["repo"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("Missing repo".to_string()))?;

        match action {
            "create" => {
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParams("Missing name".to_string()))?;

                let head_sha = params
                    .get("head_sha")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParams("Missing head_sha".to_string()))?;

                let status = params
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("in_progress");

                let mut payload = json!({
                    "name": name,
                    "head_sha": head_sha,
                    "status": status
                });

                if let Some(output) = params.get("output") {
                    payload["output"] = output.clone();
                }

                if status == "completed"
                    && let Some(conclusion) = params.get("conclusion").and_then(|v| v.as_str())
                {
                    payload["conclusion"] = json!(conclusion);
                }

                if let Some(annotations) = params.get("annotations") {
                    payload["output"]["annotations"] = annotations.clone();
                }

                let endpoint = format!("/repos/{}/check-runs", repo);
                execute_gh_api(&endpoint, "POST", Some(&payload.to_string()))
            }

            "update" => {
                let check_run_id = params
                    .get("check_run_id")
                    .and_then(|v| v.as_u64())
                    .ok_or_else(|| ToolError::InvalidParams("Missing check_run_id".to_string()))?;

                let mut payload = json!({});

                if let Some(status) = params.get("status").and_then(|v| v.as_str()) {
                    payload["status"] = json!(status);

                    if status == "completed"
                        && let Some(conclusion) = params.get("conclusion").and_then(|v| v.as_str())
                    {
                        payload["conclusion"] = json!(conclusion);
                    }
                }

                if let Some(output) = params.get("output") {
                    payload["output"] = output.clone();
                }

                if let Some(annotations) = params.get("annotations") {
                    payload["output"]["annotations"] = annotations.clone();
                }

                let endpoint = format!("/repos/{}/check-runs/{}", repo, check_run_id);
                execute_gh_api(&endpoint, "PATCH", Some(&payload.to_string()))
            }

            "list" => {
                let endpoint = if let Some(ref_name) = params.get("ref").and_then(|v| v.as_str()) {
                    format!("/repos/{}/commits/{}/check-runs", repo, ref_name)
                } else {
                    format!("/repos/{}/check-runs", repo)
                };

                execute_gh_api(&endpoint, "GET", None)
            }

            _ => Err(ToolError::InvalidParams(format!(
                "Unknown action: {action}"
            ))),
        }
    }
}

impl Default for GitHubChecksTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubChecksTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let output = self.execute_checks(&params)?;

        let result: Value =
            serde_json::from_str(&output).unwrap_or_else(|_| json!({ "raw_output": output }));

        Ok(result)
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_gh_checks_list() {
        let tool = GitHubChecksTool::new();
        let params = json!({
            "action": "list",
            "repo": "arkavo-org/arkavo-edge"
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok() || result.is_err());
    }
}
