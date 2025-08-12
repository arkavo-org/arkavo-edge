use crate::mcp::server::{Tool, ToolSchema};
use crate::{Result, TestError};
use arkavo_git::attribution::format_pr_body;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;

/// Check if GitHub CLI is installed and authenticated
fn check_gh_installed() -> Result<()> {
    let output = Command::new("gh")
        .arg("--version")
        .output()
        .map_err(|_| TestError::Mcp("GitHub CLI (gh) is not installed".to_string()))?;

    if !output.status.success() {
        return Err(TestError::Mcp(
            "GitHub CLI (gh) is not available".to_string(),
        ));
    }

    // Check authentication status
    let auth_output = Command::new("gh")
        .args(["auth", "status"])
        .output()
        .map_err(|e| TestError::Mcp(format!("Failed to check auth status: {e}")))?;

    if !auth_output.status.success() {
        return Err(TestError::Mcp(
            "GitHub CLI is not authenticated. Run 'gh auth login'".to_string(),
        ));
    }

    Ok(())
}

/// Execute a GitHub CLI command and return the output
fn execute_gh_command(args: &[String]) -> Result<String> {
    check_gh_installed()?;

    let output = Command::new("gh")
        .args(args)
        .output()
        .map_err(|e| TestError::Mcp(format!("Failed to execute gh command: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TestError::Mcp(format!(
            "GitHub CLI command failed: {stderr}"
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub struct GitHubPrCreateKit {
    schema: ToolSchema,
}

impl GitHubPrCreateKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_pr_create".to_string(),
                description: "Create a pull request on GitHub".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Title for the pull request"
                        },
                        "body": {
                            "type": "string",
                            "description": "Body/description for the pull request"
                        },
                        "base": {
                            "type": "string",
                            "description": "Base branch (defaults to repository default)"
                        },
                        "head": {
                            "type": "string",
                            "description": "Head branch (defaults to current branch)"
                        },
                        "draft": {
                            "type": "boolean",
                            "description": "Create as draft PR"
                        },
                        "assignee": {
                            "type": "string",
                            "description": "GitHub username to assign"
                        },
                        "reviewer": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "GitHub usernames to request review from"
                        },
                        "label": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Labels to add"
                        }
                    },
                    "required": ["title"]
                }),
            },
        }
    }
}

impl Default for GitHubPrCreateKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubPrCreateKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let title = params["title"]
            .as_str()
            .ok_or_else(|| TestError::Mcp("Missing title parameter".to_string()))?;

        let mut args = vec![
            "pr".to_string(),
            "create".to_string(),
            "--title".to_string(),
            title.to_string(),
        ];

        if let Some(body) = params["body"].as_str() {
            args.push("--body".to_string());
            // Format the PR body with attribution
            let formatted_body = format_pr_body(body);
            args.push(formatted_body);
        } else {
            // If no body provided, add attribution footer to empty body
            args.push("--body".to_string());
            let formatted_body = format_pr_body("");
            args.push(formatted_body);
        }

        if let Some(base) = params["base"].as_str() {
            args.push("--base".to_string());
            args.push(base.to_string());
        }

        if let Some(head) = params["head"].as_str() {
            args.push("--head".to_string());
            args.push(head.to_string());
        }

        if params["draft"].as_bool().unwrap_or(false) {
            args.push("--draft".to_string());
        }

        if let Some(assignee) = params["assignee"].as_str() {
            args.push("--assignee".to_string());
            args.push(assignee.to_string());
        }

        if let Some(reviewers) = params["reviewer"].as_array() {
            for reviewer in reviewers {
                if let Some(r) = reviewer.as_str() {
                    args.push("--reviewer".to_string());
                    args.push(r.to_string());
                }
            }
        }

        if let Some(labels) = params["label"].as_array() {
            for label in labels {
                if let Some(l) = label.as_str() {
                    args.push("--label".to_string());
                    args.push(l.to_string());
                }
            }
        }

        let output = execute_gh_command(&args)?;

        // Parse the PR URL from output
        let pr_url = output.trim();

        Ok(json!({
            "success": true,
            "pr_url": pr_url,
            "message": format!("Pull request created: {}", pr_url)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitHubPrListKit {
    schema: ToolSchema,
}

impl GitHubPrListKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_pr_list".to_string(),
                description: "List pull requests on GitHub".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "state": {
                            "type": "string",
                            "enum": ["open", "closed", "merged", "all"],
                            "description": "Filter by PR state"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of PRs to list"
                        },
                        "author": {
                            "type": "string",
                            "description": "Filter by PR author"
                        },
                        "assignee": {
                            "type": "string",
                            "description": "Filter by assignee"
                        },
                        "label": {
                            "type": "string",
                            "description": "Filter by label"
                        },
                        "json": {
                            "type": "boolean",
                            "description": "Return raw JSON output"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for GitHubPrListKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubPrListKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let mut args = vec!["pr".to_string(), "list".to_string()];

        if let Some(state) = params["state"].as_str() {
            args.push("--state".to_string());
            args.push(state.to_string());
        }

        if let Some(limit) = params["limit"].as_u64() {
            args.push("--limit".to_string());
            args.push(limit.to_string());
        }

        if let Some(author) = params["author"].as_str() {
            args.push("--author".to_string());
            args.push(author.to_string());
        }

        if let Some(assignee) = params["assignee"].as_str() {
            args.push("--assignee".to_string());
            args.push(assignee.to_string());
        }

        if let Some(label) = params["label"].as_str() {
            args.push("--label".to_string());
            args.push(label.to_string());
        }

        let return_json = params["json"].as_bool().unwrap_or(false);
        if return_json {
            args.push("--json".to_string());
            args.push("number,title,state,author,createdAt,url".to_string());
        }

        let output = execute_gh_command(&args)?;

        if return_json {
            // Parse JSON output
            let prs: Value = serde_json::from_str(&output)
                .map_err(|e| TestError::Mcp(format!("Failed to parse JSON: {e}")))?;
            Ok(json!({
                "pull_requests": prs,
                "count": prs.as_array().map(|a| a.len()).unwrap_or(0)
            }))
        } else {
            Ok(json!({
                "output": output,
                "success": true
            }))
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitHubIssueCreateKit {
    schema: ToolSchema,
}

impl GitHubIssueCreateKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_issue_create".to_string(),
                description: "Create an issue on GitHub".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Title for the issue"
                        },
                        "body": {
                            "type": "string",
                            "description": "Body/description for the issue"
                        },
                        "assignee": {
                            "type": "string",
                            "description": "GitHub username to assign"
                        },
                        "label": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Labels to add"
                        },
                        "milestone": {
                            "type": "string",
                            "description": "Milestone to assign"
                        }
                    },
                    "required": ["title"]
                }),
            },
        }
    }
}

impl Default for GitHubIssueCreateKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubIssueCreateKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let title = params["title"]
            .as_str()
            .ok_or_else(|| TestError::Mcp("Missing title parameter".to_string()))?;

        let mut args = vec![
            "issue".to_string(),
            "create".to_string(),
            "--title".to_string(),
            title.to_string(),
        ];

        if let Some(body) = params["body"].as_str() {
            args.push("--body".to_string());
            // Format the issue body with attribution
            let formatted_body = format_pr_body(body);
            args.push(formatted_body);
        } else {
            // If no body provided, add attribution footer to empty body
            args.push("--body".to_string());
            let formatted_body = format_pr_body("");
            args.push(formatted_body);
        }

        if let Some(assignee) = params["assignee"].as_str() {
            args.push("--assignee".to_string());
            args.push(assignee.to_string());
        }

        if let Some(labels) = params["label"].as_array() {
            for label in labels {
                if let Some(l) = label.as_str() {
                    args.push("--label".to_string());
                    args.push(l.to_string());
                }
            }
        }

        if let Some(milestone) = params["milestone"].as_str() {
            args.push("--milestone".to_string());
            args.push(milestone.to_string());
        }

        let output = execute_gh_command(&args)?;

        // Parse the issue URL from output
        let issue_url = output.trim();

        Ok(json!({
            "success": true,
            "issue_url": issue_url,
            "message": format!("Issue created: {}", issue_url)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitHubIssueListKit {
    schema: ToolSchema,
}

impl GitHubIssueListKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_issue_list".to_string(),
                description: "List issues on GitHub".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "state": {
                            "type": "string",
                            "enum": ["open", "closed", "all"],
                            "description": "Filter by issue state"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of issues to list"
                        },
                        "author": {
                            "type": "string",
                            "description": "Filter by issue author"
                        },
                        "assignee": {
                            "type": "string",
                            "description": "Filter by assignee"
                        },
                        "label": {
                            "type": "string",
                            "description": "Filter by label"
                        },
                        "json": {
                            "type": "boolean",
                            "description": "Return raw JSON output"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for GitHubIssueListKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubIssueListKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let mut args = vec!["issue".to_string(), "list".to_string()];

        if let Some(state) = params["state"].as_str() {
            args.push("--state".to_string());
            args.push(state.to_string());
        }

        if let Some(limit) = params["limit"].as_u64() {
            args.push("--limit".to_string());
            args.push(limit.to_string());
        }

        if let Some(author) = params["author"].as_str() {
            args.push("--author".to_string());
            args.push(author.to_string());
        }

        if let Some(assignee) = params["assignee"].as_str() {
            args.push("--assignee".to_string());
            args.push(assignee.to_string());
        }

        if let Some(label) = params["label"].as_str() {
            args.push("--label".to_string());
            args.push(label.to_string());
        }

        let return_json = params["json"].as_bool().unwrap_or(false);
        if return_json {
            args.push("--json".to_string());
            args.push("number,title,state,author,createdAt,url".to_string());
        }

        let output = execute_gh_command(&args)?;

        if return_json {
            // Parse JSON output
            let issues: Value = serde_json::from_str(&output)
                .map_err(|e| TestError::Mcp(format!("Failed to parse JSON: {e}")))?;
            Ok(json!({
                "issues": issues,
                "count": issues.as_array().map(|a| a.len()).unwrap_or(0)
            }))
        } else {
            Ok(json!({
                "output": output,
                "success": true
            }))
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitHubRepoCloneKit {
    schema: ToolSchema,
}

impl GitHubRepoCloneKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_repo_clone".to_string(),
                description: "Clone a GitHub repository".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "repository": {
                            "type": "string",
                            "description": "Repository in format owner/repo"
                        },
                        "path": {
                            "type": "string",
                            "description": "Local path to clone to"
                        },
                        "shallow": {
                            "type": "boolean",
                            "description": "Perform shallow clone (depth 1)"
                        }
                    },
                    "required": ["repository"]
                }),
            },
        }
    }
}

impl Default for GitHubRepoCloneKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubRepoCloneKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let repository = params["repository"]
            .as_str()
            .ok_or_else(|| TestError::Mcp("Missing repository parameter".to_string()))?;

        let mut args = vec![
            "repo".to_string(),
            "clone".to_string(),
            repository.to_string(),
        ];

        if let Some(path) = params["path"].as_str() {
            args.push(path.to_string());
        }

        if params["shallow"].as_bool().unwrap_or(false) {
            args.push("--".to_string());
            args.push("--depth".to_string());
            args.push("1".to_string());
        }

        let output = execute_gh_command(&args)?;

        Ok(json!({
            "success": true,
            "repository": repository,
            "message": format!("Repository {} cloned successfully", repository),
            "output": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitHubReleaseCreateKit {
    schema: ToolSchema,
}

impl GitHubReleaseCreateKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_release_create".to_string(),
                description: "Create a GitHub release".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "tag": {
                            "type": "string",
                            "description": "Tag name for the release"
                        },
                        "title": {
                            "type": "string",
                            "description": "Title for the release"
                        },
                        "notes": {
                            "type": "string",
                            "description": "Release notes"
                        },
                        "draft": {
                            "type": "boolean",
                            "description": "Create as draft release"
                        },
                        "prerelease": {
                            "type": "boolean",
                            "description": "Mark as prerelease"
                        },
                        "target": {
                            "type": "string",
                            "description": "Target branch or commit SHA"
                        }
                    },
                    "required": ["tag"]
                }),
            },
        }
    }
}

impl Default for GitHubReleaseCreateKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubReleaseCreateKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let tag = params["tag"]
            .as_str()
            .ok_or_else(|| TestError::Mcp("Missing tag parameter".to_string()))?;

        let mut args = vec!["release".to_string(), "create".to_string(), tag.to_string()];

        if let Some(title) = params["title"].as_str() {
            args.push("--title".to_string());
            args.push(title.to_string());
        }

        if let Some(notes) = params["notes"].as_str() {
            args.push("--notes".to_string());
            args.push(notes.to_string());
        }

        if params["draft"].as_bool().unwrap_or(false) {
            args.push("--draft".to_string());
        }

        if params["prerelease"].as_bool().unwrap_or(false) {
            args.push("--prerelease".to_string());
        }

        if let Some(target) = params["target"].as_str() {
            args.push("--target".to_string());
            args.push(target.to_string());
        }

        let output = execute_gh_command(&args)?;

        // Parse the release URL from output
        let release_url = output.trim();

        Ok(json!({
            "success": true,
            "release_url": release_url,
            "tag": tag,
            "message": format!("Release {} created: {}", tag, release_url)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitHubPrMergeKit {
    schema: ToolSchema,
}

impl GitHubPrMergeKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_pr_merge".to_string(),
                description: "Merge a pull request on GitHub".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "number": {
                            "type": "integer",
                            "description": "PR number to merge"
                        },
                        "method": {
                            "type": "string",
                            "enum": ["merge", "squash", "rebase"],
                            "description": "Merge method (defaults to merge)"
                        },
                        "auto": {
                            "type": "boolean",
                            "description": "Enable auto-merge"
                        },
                        "delete_branch": {
                            "type": "boolean",
                            "description": "Delete branch after merge"
                        }
                    },
                    "required": ["number"]
                }),
            },
        }
    }
}

impl Default for GitHubPrMergeKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubPrMergeKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let number = params["number"]
            .as_u64()
            .ok_or_else(|| TestError::Mcp("Missing number parameter".to_string()))?;

        let mut args = vec!["pr".to_string(), "merge".to_string(), number.to_string()];

        if let Some(method) = params["method"].as_str() {
            args.push(match method {
                "squash" => "--squash".to_string(),
                "rebase" => "--rebase".to_string(),
                _ => "--merge".to_string(),
            });
        }

        if params["auto"].as_bool().unwrap_or(false) {
            args.push("--auto".to_string());
        }

        if params["delete_branch"].as_bool().unwrap_or(false) {
            args.push("--delete-branch".to_string());
        }

        let output = execute_gh_command(&args)?;

        Ok(json!({
            "success": true,
            "pr_number": number,
            "message": format!("Pull request #{} merged successfully", number),
            "output": output
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_schemas() {
        // Test that all tools have valid schemas
        let pr_create = GitHubPrCreateKit::new();
        assert_eq!(pr_create.schema().name, "github_pr_create");

        let pr_list = GitHubPrListKit::new();
        assert_eq!(pr_list.schema().name, "github_pr_list");

        let issue_create = GitHubIssueCreateKit::new();
        assert_eq!(issue_create.schema().name, "github_issue_create");

        let issue_list = GitHubIssueListKit::new();
        assert_eq!(issue_list.schema().name, "github_issue_list");

        let repo_clone = GitHubRepoCloneKit::new();
        assert_eq!(repo_clone.schema().name, "github_repo_clone");

        let release_create = GitHubReleaseCreateKit::new();
        assert_eq!(release_create.schema().name, "github_release_create");

        let pr_merge = GitHubPrMergeKit::new();
        assert_eq!(pr_merge.schema().name, "github_pr_merge");
    }

    #[test]
    fn test_pr_body_formatting() {
        // Test that PR body gets attribution footer
        let pr_create = GitHubPrCreateKit::new();

        // Simulate parameters with body
        let params_with_body = json!({
            "title": "Test PR",
            "body": "This is a test PR description"
        });

        // We can't easily test the execute method without mocking gh,
        // but we can test the attribution formatting directly
        let formatted_body = format_pr_body("This is a test PR description");
        assert!(formatted_body.contains("This is a test PR description"));
        assert!(formatted_body.contains("🤖 Generated with [Arkavo Edge]"));
        assert!(formatted_body.contains("https://arkavo.ai/edge"));

        // Test empty body
        let formatted_empty = format_pr_body("");
        assert!(formatted_empty.contains("🤖 Generated with [Arkavo Edge]"));
    }

    #[test]
    fn test_issue_body_formatting() {
        // Test that issue body gets attribution footer
        let formatted_body = format_pr_body("This is a test issue description");
        assert!(formatted_body.contains("This is a test issue description"));
        assert!(formatted_body.contains("🤖 Generated with [Arkavo Edge]"));
        assert!(formatted_body.contains("https://arkavo.ai/edge"));
    }
}
