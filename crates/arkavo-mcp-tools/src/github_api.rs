//! GitHub MCP tools using direct API calls via reqwest
//!
//! This module uses the GitHub REST API directly instead of a third-party client.

use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use arkavo_git::attribution::format_pr_body;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::OnceCell;

pub(crate) const GITHUB_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = "Arkavo-MCP-Tools/0.1";

/// Merge method for pull requests
#[derive(Debug, Clone, Copy)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

/// Commit ref returned inside a PR response.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GhHead {
    pub(crate) sha: String,
}

/// Minimal PR response
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GhPullRequest {
    pub(crate) number: u64,
    pub(crate) title: Option<String>,
    pub(crate) html_url: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) user: Option<GhUser>,
    pub(crate) head: Option<GhHead>,
    pub(crate) updated_at: Option<String>,
}

/// Minimal issue response
#[derive(Debug, Deserialize)]
struct GhIssue {
    number: u64,
    title: String,
    html_url: String,
    state: String,
    user: GhUser,
}

/// Minimal release response
#[derive(Debug, Deserialize)]
struct GhRelease {
    html_url: String,
}

/// Minimal user response
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct GhUser {
    pub(crate) login: String,
}

pub(crate) struct GitHubClient {
    pub(crate) client: reqwest::Client,
    token: String,
}

/// Global GitHub client (lazily initialized from GITHUB_TOKEN)
static GITHUB_CLIENT: OnceCell<GitHubClient> = OnceCell::const_new();

/// Get or initialize the GitHub client
pub(crate) async fn get_github_client() -> Result<&'static GitHubClient> {
    GITHUB_CLIENT
        .get_or_try_init(|| async {
            let token = std::env::var("GITHUB_TOKEN")
                .map_err(|_| ToolError::Mcp("GITHUB_TOKEN environment variable not set".into()))?;

            let client = reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .map_err(|e| ToolError::Mcp(format!("Failed to create HTTP client: {e}")))?;

            Ok(GitHubClient { client, token })
        })
        .await
}

/// Send a GitHub API request and check for errors
pub(crate) async fn github_request(
    gh: &GitHubClient,
    request: reqwest::RequestBuilder,
    action: &str,
) -> Result<reqwest::Response> {
    let resp = request
        .header("Authorization", format!("Bearer {}", gh.token))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| ToolError::Mcp(format!("Failed to {action}: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(ToolError::Mcp(format!(
            "Failed to {action} ({status}): {text}"
        )));
    }

    Ok(resp)
}

/// Extract owner and repo from a "owner/repo" format string
fn parse_repo(repo_str: &str) -> Result<(&str, &str)> {
    let parts: Vec<&str> = repo_str.split('/').collect();
    if parts.len() != 2 {
        return Err(ToolError::Mcp(format!(
            "Invalid repository format: '{}'. Expected 'owner/repo'",
            repo_str
        )));
    }
    Ok((parts[0], parts[1]))
}

/// Get current repository from git config
fn get_current_repo() -> Result<(String, String)> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|e| ToolError::Mcp(format!("Failed to get git remote: {e}")))?;

    if !output.status.success() {
        return Err(ToolError::Mcp(
            "Not in a git repository or no remote configured".to_string(),
        ));
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Parse GitHub URL formats:
    // https://github.com/owner/repo.git
    // git@github.com:owner/repo.git
    let repo_path = if url.starts_with("https://github.com/") {
        url.trim_start_matches("https://github.com/")
            .trim_end_matches(".git")
    } else if url.starts_with("git@github.com:") {
        url.trim_start_matches("git@github.com:")
            .trim_end_matches(".git")
    } else {
        return Err(ToolError::Mcp(format!(
            "Unsupported remote URL format: {}",
            url
        )));
    };

    let (owner, repo) = parse_repo(repo_path)?;
    Ok((owner.to_string(), repo.to_string()))
}

/// Get current branch name
fn get_current_branch() -> Result<String> {
    use std::process::Command;

    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .map_err(|e| ToolError::Mcp(format!("Failed to get current branch: {e}")))?;

    if !output.status.success() {
        return Err(ToolError::Mcp("Failed to get current branch".to_string()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ============================================================================
// PR Create Tool
// ============================================================================

pub struct GitHubPrCreateTool {
    schema: ToolSchema,
}

impl GitHubPrCreateTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_pr_create".to_string(),
                aliases: None,
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
                            "description": "Base branch (defaults to 'main')"
                        },
                        "head": {
                            "type": "string",
                            "description": "Head branch (defaults to current branch)"
                        },
                        "repository": {
                            "type": "string",
                            "description": "Repository in format 'owner/repo' (defaults to current repo)"
                        }
                    },
                    "required": ["title"]
                }),
            },
        }
    }
}

impl Default for GitHubPrCreateTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubPrCreateTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let gh = get_github_client().await?;

        let title = params["title"]
            .as_str()
            .ok_or_else(|| ToolError::Mcp("Missing title parameter".to_string()))?;

        let (owner, repo) = if let Some(repo_str) = params["repository"].as_str() {
            let (o, r) = parse_repo(repo_str)?;
            (o.to_string(), r.to_string())
        } else {
            get_current_repo()?
        };

        let base = params["base"].as_str().unwrap_or("main");
        let head = if let Some(h) = params["head"].as_str() {
            h.to_string()
        } else {
            get_current_branch()?
        };

        let body = params["body"].as_str().unwrap_or("");
        let formatted_body = format_pr_body(body);

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls");
        let resp = github_request(
            gh,
            gh.client.post(&url).json(&json!({
                "title": title,
                "body": formatted_body,
                "head": head,
                "base": base
            })),
            "create PR",
        )
        .await?;

        let pr: GhPullRequest = resp
            .json()
            .await
            .map_err(|e| ToolError::Mcp(format!("Failed to parse PR response: {e}")))?;

        let pr_url = pr
            .html_url
            .unwrap_or_else(|| format!("https://github.com/{}/{}/pull/{}", owner, repo, pr.number));

        Ok(json!({
            "success": true,
            "pr_url": pr_url,
            "pr_number": pr.number,
            "message": format!("Pull request #{} created: {}", pr.number, pr_url)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// PR List Tool
// ============================================================================

pub struct GitHubPrListTool {
    schema: ToolSchema,
}

impl GitHubPrListTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_pr_list".to_string(),
                aliases: None,
                description: "List pull requests on GitHub".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "state": {
                            "type": "string",
                            "enum": ["open", "closed", "all"],
                            "description": "Filter by PR state"
                        },
                        "repository": {
                            "type": "string",
                            "description": "Repository in format 'owner/repo' (defaults to current repo)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for GitHubPrListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubPrListTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let gh = get_github_client().await?;

        let (owner, repo) = if let Some(repo_str) = params["repository"].as_str() {
            let (o, r) = parse_repo(repo_str)?;
            (o.to_string(), r.to_string())
        } else {
            get_current_repo()?
        };

        let state = match params["state"].as_str() {
            Some("closed") => "closed",
            Some("all") => "all",
            _ => "open",
        };

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls?state={state}");
        let resp = github_request(gh, gh.client.get(&url), "list PRs").await?;

        let prs: Vec<GhPullRequest> = resp
            .json()
            .await
            .map_err(|e| ToolError::Mcp(format!("Failed to parse PR list: {e}")))?;

        let pr_list: Vec<Value> = prs
            .iter()
            .map(|pr| {
                json!({
                    "number": pr.number,
                    "title": pr.title.as_deref().unwrap_or(""),
                    "state": pr.state.as_deref().unwrap_or("unknown"),
                    "url": pr.html_url.as_deref().unwrap_or(""),
                    "user": pr.user.as_ref().map(|u| u.login.clone()).unwrap_or_default()
                })
            })
            .collect();

        Ok(json!({
            "pull_requests": pr_list,
            "count": pr_list.len()
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// PR Merge Tool
// ============================================================================

pub struct GitHubPrMergeTool {
    schema: ToolSchema,
}

impl GitHubPrMergeTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_pr_merge".to_string(),
                aliases: None,
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
                        "repository": {
                            "type": "string",
                            "description": "Repository in format 'owner/repo' (defaults to current repo)"
                        }
                    },
                    "required": ["number"]
                }),
            },
        }
    }
}

impl Default for GitHubPrMergeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubPrMergeTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let gh = get_github_client().await?;

        let number = params["number"]
            .as_u64()
            .ok_or_else(|| ToolError::Mcp("Missing number parameter".to_string()))?;

        let (owner, repo) = if let Some(repo_str) = params["repository"].as_str() {
            let (o, r) = parse_repo(repo_str)?;
            (o.to_string(), r.to_string())
        } else {
            get_current_repo()?
        };

        let merge_method = match params["method"].as_str() {
            Some("squash") => "squash",
            Some("rebase") => "rebase",
            _ => "merge",
        };

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls/{number}/merge");
        github_request(
            gh,
            gh.client
                .put(&url)
                .json(&json!({ "merge_method": merge_method })),
            "merge PR",
        )
        .await?;

        Ok(json!({
            "success": true,
            "pr_number": number,
            "message": format!("Pull request #{} merged successfully", number)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// Issue Create Tool
// ============================================================================

pub struct GitHubIssueCreateTool {
    schema: ToolSchema,
}

impl GitHubIssueCreateTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_issue_create".to_string(),
                aliases: None,
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
                        "repository": {
                            "type": "string",
                            "description": "Repository in format 'owner/repo' (defaults to current repo)"
                        }
                    },
                    "required": ["title"]
                }),
            },
        }
    }
}

impl Default for GitHubIssueCreateTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubIssueCreateTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let gh = get_github_client().await?;

        let title = params["title"]
            .as_str()
            .ok_or_else(|| ToolError::Mcp("Missing title parameter".to_string()))?;

        let (owner, repo) = if let Some(repo_str) = params["repository"].as_str() {
            let (o, r) = parse_repo(repo_str)?;
            (o.to_string(), r.to_string())
        } else {
            get_current_repo()?
        };

        let body = params["body"].as_str().unwrap_or("");
        let formatted_body = format_pr_body(body);

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues");
        let resp = github_request(
            gh,
            gh.client.post(&url).json(&json!({
                "title": title,
                "body": formatted_body
            })),
            "create issue",
        )
        .await?;

        let issue: GhIssue = resp
            .json()
            .await
            .map_err(|e| ToolError::Mcp(format!("Failed to parse issue response: {e}")))?;

        Ok(json!({
            "success": true,
            "issue_url": issue.html_url,
            "issue_number": issue.number,
            "message": format!("Issue #{} created: {}", issue.number, issue.html_url)
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// Issue List Tool
// ============================================================================

pub struct GitHubIssueListTool {
    schema: ToolSchema,
}

impl GitHubIssueListTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_issue_list".to_string(),
                aliases: None,
                description: "List issues on GitHub".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "state": {
                            "type": "string",
                            "enum": ["open", "closed", "all"],
                            "description": "Filter by issue state"
                        },
                        "repository": {
                            "type": "string",
                            "description": "Repository in format 'owner/repo' (defaults to current repo)"
                        }
                    }
                }),
            },
        }
    }
}

impl Default for GitHubIssueListTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubIssueListTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let gh = get_github_client().await?;

        let (owner, repo) = if let Some(repo_str) = params["repository"].as_str() {
            let (o, r) = parse_repo(repo_str)?;
            (o.to_string(), r.to_string())
        } else {
            get_current_repo()?
        };

        let state = match params["state"].as_str() {
            Some("closed") => "closed",
            Some("all") => "all",
            _ => "open",
        };

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues?state={state}");
        let resp = github_request(gh, gh.client.get(&url), "list issues").await?;

        let issues: Vec<GhIssue> = resp
            .json()
            .await
            .map_err(|e| ToolError::Mcp(format!("Failed to parse issue list: {e}")))?;

        let issue_list: Vec<Value> = issues
            .iter()
            .map(|issue| {
                json!({
                    "number": issue.number,
                    "title": &issue.title,
                    "state": &issue.state,
                    "url": &issue.html_url,
                    "user": issue.user.login.clone()
                })
            })
            .collect();

        Ok(json!({
            "issues": issue_list,
            "count": issue_list.len()
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

// ============================================================================
// Release Create Tool
// ============================================================================

pub struct GitHubReleaseCreateTool {
    schema: ToolSchema,
}

impl GitHubReleaseCreateTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_release_create".to_string(),
                aliases: None,
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
                        "repository": {
                            "type": "string",
                            "description": "Repository in format 'owner/repo' (defaults to current repo)"
                        }
                    },
                    "required": ["tag"]
                }),
            },
        }
    }
}

impl Default for GitHubReleaseCreateTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubReleaseCreateTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let gh = get_github_client().await?;

        let tag = params["tag"]
            .as_str()
            .ok_or_else(|| ToolError::Mcp("Missing tag parameter".to_string()))?;

        let (owner, repo) = if let Some(repo_str) = params["repository"].as_str() {
            let (o, r) = parse_repo(repo_str)?;
            (o.to_string(), r.to_string())
        } else {
            get_current_repo()?
        };

        let title = params["title"].as_str().unwrap_or(tag);
        let notes = params["notes"].as_str().unwrap_or("");

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/releases");
        let resp = github_request(
            gh,
            gh.client.post(&url).json(&json!({
                "tag_name": tag,
                "name": title,
                "body": notes
            })),
            "create release",
        )
        .await?;

        let release: GhRelease = resp
            .json()
            .await
            .map_err(|e| ToolError::Mcp(format!("Failed to parse release response: {e}")))?;

        Ok(json!({
            "success": true,
            "release_url": release.html_url,
            "tag": tag,
            "message": format!("Release {} created: {}", tag, release.html_url)
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
    fn test_parse_repo() {
        let (owner, repo) = parse_repo("arkavo-org/arkavo-edge").unwrap();
        assert_eq!(owner, "arkavo-org");
        assert_eq!(repo, "arkavo-edge");
    }

    #[test]
    fn test_parse_repo_invalid() {
        assert!(parse_repo("invalid").is_err());
        assert!(parse_repo("a/b/c").is_err());
    }

    #[test]
    fn test_tool_schemas() {
        let pr_create = GitHubPrCreateTool::new();
        assert_eq!(pr_create.schema().name, "github_pr_create");

        let pr_list = GitHubPrListTool::new();
        assert_eq!(pr_list.schema().name, "github_pr_list");

        let pr_merge = GitHubPrMergeTool::new();
        assert_eq!(pr_merge.schema().name, "github_pr_merge");

        let issue_create = GitHubIssueCreateTool::new();
        assert_eq!(issue_create.schema().name, "github_issue_create");

        let issue_list = GitHubIssueListTool::new();
        assert_eq!(issue_list.schema().name, "github_issue_list");

        let release_create = GitHubReleaseCreateTool::new();
        assert_eq!(release_create.schema().name, "github_release_create");
    }
}
