use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::process::Command;

pub struct GitHubOrgReposTool {
    schema: ToolSchema,
}

impl Default for GitHubOrgReposTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHubOrgReposTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_org_repos".to_string(),
                description: "List all repositories in a GitHub organization with health metrics"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "org": {
                            "type": "string",
                            "description": "GitHub organization name"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of repos to return (default: 100)"
                        }
                    },
                    "required": ["org"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for GitHubOrgReposTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let org = params["org"]
            .as_str()
            .ok_or_else(|| ToolError::Mcp("Missing required parameter: org".to_string()))?;
        let limit = params["limit"].as_i64().unwrap_or(100);

        let output = Command::new("gh")
            .args([
                "repo",
                "list",
                org,
                "--limit",
                &limit.to_string(),
                "--json",
                "name,description,updatedAt,isPrivate,isFork,isArchived,stargazerCount,forkCount,primaryLanguage,url",
            ])
            .output()
            .map_err(|e| ToolError::Execution(format!("Failed to execute gh command: {e}")))?;

        if !output.status.success() {
            return Err(ToolError::Execution(format!(
                "gh command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let repos: Value = serde_json::from_slice(&output.stdout).map_err(ToolError::Json)?;

        Ok(json!({
            "org": org,
            "count": repos.as_array().map(|a| a.len()).unwrap_or(0),
            "repos": repos
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitHubRelatedIssuesTool {
    schema: ToolSchema,
}

impl Default for GitHubRelatedIssuesTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHubRelatedIssuesTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_related_issues".to_string(),
                description:
                    "Find issues related to a given issue (linked, mentioned, or blocking)"
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "owner": {
                            "type": "string",
                            "description": "Repository owner"
                        },
                        "repo": {
                            "type": "string",
                            "description": "Repository name"
                        },
                        "issue_number": {
                            "type": "integer",
                            "description": "Issue number to find related issues for"
                        }
                    },
                    "required": ["owner", "repo", "issue_number"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for GitHubRelatedIssuesTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let owner = params["owner"]
            .as_str()
            .ok_or_else(|| ToolError::Mcp("Missing required parameter: owner".to_string()))?;
        let repo = params["repo"]
            .as_str()
            .ok_or_else(|| ToolError::Mcp("Missing required parameter: repo".to_string()))?;
        let issue_number = params["issue_number"].as_i64().ok_or_else(|| {
            ToolError::Mcp("Missing required parameter: issue_number".to_string())
        })?;

        let output = Command::new("gh")
            .args([
                "issue",
                "view",
                &issue_number.to_string(),
                "--repo",
                &format!("{}/{}", owner, repo),
                "--json",
                "body,comments",
            ])
            .output()
            .map_err(|e| ToolError::Execution(format!("Failed to execute gh command: {e}")))?;

        if !output.status.success() {
            return Err(ToolError::Execution(format!(
                "gh command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let issue_data: Value = serde_json::from_slice(&output.stdout).map_err(ToolError::Json)?;

        let mut related_issues = Vec::new();
        let issue_pattern = regex::Regex::new(r"#(\d+)").unwrap();
        let full_pattern = regex::Regex::new(r"([a-zA-Z0-9_-]+)/([a-zA-Z0-9_-]+)#(\d+)").unwrap();

        if let Some(body) = issue_data["body"].as_str() {
            for cap in issue_pattern.captures_iter(body) {
                if let Some(num) = cap.get(1) {
                    related_issues.push(json!({
                        "type": "same_repo",
                        "issue_number": num.as_str().parse::<i64>().unwrap_or(0)
                    }));
                }
            }

            for cap in full_pattern.captures_iter(body) {
                if let (Some(o), Some(r), Some(num)) = (cap.get(1), cap.get(2), cap.get(3)) {
                    related_issues.push(json!({
                        "type": "cross_repo",
                        "owner": o.as_str(),
                        "repo": r.as_str(),
                        "issue_number": num.as_str().parse::<i64>().unwrap_or(0)
                    }));
                }
            }
        }

        if let Some(comments) = issue_data["comments"].as_array() {
            for comment in comments {
                if let Some(body) = comment["body"].as_str() {
                    for cap in issue_pattern.captures_iter(body) {
                        if let Some(num) = cap.get(1) {
                            related_issues.push(json!({
                                "type": "same_repo",
                                "issue_number": num.as_str().parse::<i64>().unwrap_or(0)
                            }));
                        }
                    }
                }
            }
        }

        related_issues.sort_by(|a, b| {
            a["issue_number"]
                .as_i64()
                .unwrap_or(0)
                .cmp(&b["issue_number"].as_i64().unwrap_or(0))
        });
        related_issues.dedup();

        Ok(json!({
            "owner": owner,
            "repo": repo,
            "issue_number": issue_number,
            "related_issues": related_issues
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitHubCiStatusTool {
    schema: ToolSchema,
}

impl Default for GitHubCiStatusTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHubCiStatusTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_ci_status".to_string(),
                description: "Get CI/workflow status for a repository or pull request".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "owner": {
                            "type": "string",
                            "description": "Repository owner"
                        },
                        "repo": {
                            "type": "string",
                            "description": "Repository name"
                        },
                        "pr_number": {
                            "type": "integer",
                            "description": "Optional PR number to get checks for specific PR"
                        },
                        "branch": {
                            "type": "string",
                            "description": "Optional branch name to get workflow runs for"
                        }
                    },
                    "required": ["owner", "repo"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for GitHubCiStatusTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let owner = params["owner"]
            .as_str()
            .ok_or_else(|| ToolError::Mcp("Missing required parameter: owner".to_string()))?;
        let repo = params["repo"]
            .as_str()
            .ok_or_else(|| ToolError::Mcp("Missing required parameter: repo".to_string()))?;
        let repo_full = format!("{}/{}", owner, repo);

        if let Some(pr_number) = params["pr_number"].as_i64() {
            let output = Command::new("gh")
                .args([
                    "pr",
                    "checks",
                    &pr_number.to_string(),
                    "--repo",
                    &repo_full,
                    "--json",
                    "bucket,name,status,conclusion,detailsUrl",
                ])
                .output()
                .map_err(|e| ToolError::Execution(format!("Failed to execute gh command: {e}")))?;

            if !output.status.success() {
                return Err(ToolError::Execution(format!(
                    "gh command failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                )));
            }

            let checks: Value = serde_json::from_slice(&output.stdout).map_err(ToolError::Json)?;

            return Ok(json!({
                "type": "pr_checks",
                "owner": owner,
                "repo": repo,
                "pr_number": pr_number,
                "checks": checks
            }));
        }

        let mut args = vec![
            "run",
            "list",
            "--repo",
            &repo_full,
            "--json",
            "name,status,conclusion,createdAt,event,headBranch",
        ];

        if let Some(branch) = params["branch"].as_str() {
            args.extend(["--branch", branch]);
        }

        args.extend(["--limit", "10"]);

        let output = Command::new("gh")
            .args(&args)
            .output()
            .map_err(|e| ToolError::Execution(format!("Failed to execute gh command: {e}")))?;

        if !output.status.success() {
            return Err(ToolError::Execution(format!(
                "gh command failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let runs: Value = serde_json::from_slice(&output.stdout).map_err(ToolError::Json)?;

        Ok(json!({
            "type": "workflow_runs",
            "owner": owner,
            "repo": repo,
            "branch": params["branch"],
            "runs": runs
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct GitHubOrgOverviewTool {
    schema: ToolSchema,
}

impl Default for GitHubOrgOverviewTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHubOrgOverviewTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_org_overview".to_string(),
                description:
                    "Get organization-wide overview: open issues, PRs, failed CI across repos"
                        .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "org": {
                            "type": "string",
                            "description": "GitHub organization name"
                        }
                    },
                    "required": ["org"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for GitHubOrgOverviewTool {
    async fn execute(&self, params: Value) -> Result<Value> {
        let org = params["org"]
            .as_str()
            .ok_or_else(|| ToolError::Mcp("Missing required parameter: org".to_string()))?;

        let repos_output = Command::new("gh")
            .args([
                "repo",
                "list",
                org,
                "--limit",
                "100",
                "--json",
                "name,isArchived",
            ])
            .output()
            .map_err(|e| ToolError::Execution(format!("Failed to execute gh command: {e}")))?;

        if !repos_output.status.success() {
            return Err(ToolError::Execution(format!(
                "gh command failed: {}",
                String::from_utf8_lossy(&repos_output.stderr)
            )));
        }

        let repos: Vec<Value> =
            serde_json::from_slice(&repos_output.stdout).map_err(ToolError::Json)?;

        let active_repos: Vec<&Value> = repos
            .iter()
            .filter(|r| !r["isArchived"].as_bool().unwrap_or(false))
            .collect();

        let mut total_issues = 0;
        let mut total_prs = 0;

        for repo in &active_repos {
            if let Some(repo_name) = repo["name"].as_str() {
                let repo_full = format!("{}/{}", org, repo_name);

                if let Ok(issues_output) = Command::new("gh")
                    .args([
                        "issue", "list", "--repo", &repo_full, "--state", "open", "--json",
                        "number",
                    ])
                    .output()
                    && issues_output.status.success()
                    && let Ok(issues) = serde_json::from_slice::<Vec<Value>>(&issues_output.stdout)
                {
                    total_issues += issues.len();
                }

                if let Ok(prs_output) = Command::new("gh")
                    .args([
                        "pr", "list", "--repo", &repo_full, "--state", "open", "--json", "number",
                    ])
                    .output()
                    && prs_output.status.success()
                    && let Ok(prs) = serde_json::from_slice::<Vec<Value>>(&prs_output.stdout)
                {
                    total_prs += prs.len();
                }
            }
        }

        Ok(json!({
            "org": org,
            "total_repos": repos.len(),
            "active_repos": active_repos.len(),
            "archived_repos": repos.len() - active_repos.len(),
            "total_open_issues": total_issues,
            "total_open_prs": total_prs,
            "summary": format!(
                "{} has {} active repos with {} open issues and {} open PRs",
                org,
                active_repos.len(),
                total_issues,
                total_prs
            )
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
