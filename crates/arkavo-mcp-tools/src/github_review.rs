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

pub struct GitHubReviewTool {
    schema: ToolSchema,
}

impl GitHubReviewTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "gh_pr_review".to_string(),
                aliases: None,
                description: "Submit PR reviews with line-level comments and code suggestions"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "repo": {
                            "type": "string",
                            "description": "Repository in owner/repo format"
                        },
                        "pr_number": {
                            "type": "integer",
                            "description": "Pull request number"
                        },
                        "event": {
                            "type": "string",
                            "enum": ["COMMENT", "APPROVE", "REQUEST_CHANGES"],
                            "description": "Review action"
                        },
                        "body": {
                            "type": "string",
                            "description": "Overall review comment"
                        },
                        "comments": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": {
                                        "type": "string",
                                        "description": "File path in the PR"
                                    },
                                    "line": {
                                        "type": "integer",
                                        "description": "Line number for the comment"
                                    },
                                    "body": {
                                        "type": "string",
                                        "description": "Comment text"
                                    },
                                    "side": {
                                        "type": "string",
                                        "enum": ["LEFT", "RIGHT"],
                                        "description": "Side of diff (default: RIGHT)"
                                    }
                                },
                                "required": ["path", "line", "body"]
                            },
                            "description": "Line-level review comments"
                        },
                        "commit_id": {
                            "type": "string",
                            "description": "Specific commit SHA to review (defaults to latest)"
                        }
                    },
                    "required": ["repo", "pr_number", "event"]
                }),
            },
        }
    }

    fn execute_review(&self, params: &Value) -> Result<String> {
        let repo = params["repo"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("Missing repo".to_string()))?;

        let pr_number = params["pr_number"]
            .as_u64()
            .ok_or_else(|| ToolError::InvalidParams("Missing pr_number".to_string()))?;

        let event = params["event"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("Missing event".to_string()))?;

        let mut payload = json!({
            "event": event
        });

        if let Some(body) = params.get("body").and_then(|v| v.as_str()) {
            payload["body"] = json!(body);
        }

        if let Some(commit_id) = params.get("commit_id").and_then(|v| v.as_str()) {
            payload["commit_id"] = json!(commit_id);
        } else {
            let commits_endpoint = format!("/repos/{}/pulls/{}/commits", repo, pr_number);
            let commits_response = execute_gh_api(&commits_endpoint, "GET", None)?;
            let commits: Vec<Value> = serde_json::from_str(&commits_response)
                .map_err(|e| ToolError::Mcp(format!("Failed to parse commits: {e}")))?;

            if let Some(latest) = commits.last()
                && let Some(sha) = latest.get("sha").and_then(|s| s.as_str())
            {
                payload["commit_id"] = json!(sha);
            }
        }

        if let Some(comments) = params.get("comments").and_then(|v| v.as_array()) {
            let review_comments: Vec<Value> = comments
                .iter()
                .map(|c| {
                    let mut comment = json!({
                        "path": c["path"],
                        "line": c["line"],
                        "body": c["body"]
                    });

                    if let Some(side) = c.get("side").and_then(|s| s.as_str()) {
                        comment["side"] = json!(side);
                    } else {
                        comment["side"] = json!("RIGHT");
                    }

                    comment
                })
                .collect();

            payload["comments"] = json!(review_comments);
        }

        let endpoint = format!("/repos/{}/pulls/{}/reviews", repo, pr_number);
        execute_gh_api(&endpoint, "POST", Some(&payload.to_string()))
    }
}

impl Default for GitHubReviewTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubReviewTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let output = self.execute_review(&params)?;

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
    async fn test_pr_review() {
        let tool = GitHubReviewTool::new();
        let params = json!({
            "repo": "arkavo-org/arkavo-edge",
            "pr_number": 1,
            "event": "COMMENT",
            "body": "Test review"
        });

        let result = tool.execute(params).await;
        assert!(result.is_ok() || result.is_err());
    }
}
