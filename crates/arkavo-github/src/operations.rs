//! GitHub API operations for PRs, Issues, and Releases
//!
//! Provides direct API access via reqwest, using the GitHub REST API.

use crate::error::{GitHubError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

const GITHUB_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("Arkavo-GitHub/", env!("CARGO_PKG_VERSION"));

/// Minimal pull request response from GitHub API
#[derive(Debug, Clone, Deserialize)]
pub struct GhPullRequest {
    pub number: u64,
    pub title: Option<String>,
    pub html_url: Option<String>,
    pub state: Option<String>,
    pub user: Option<GhUser>,
}

/// Minimal issue response from GitHub API
#[derive(Debug, Clone, Deserialize)]
pub struct GhIssue {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    pub state: String,
    pub user: GhUser,
}

/// Minimal release response from GitHub API
#[derive(Debug, Clone, Deserialize)]
pub struct GhRelease {
    pub id: u64,
    pub tag_name: String,
    pub name: Option<String>,
    pub html_url: String,
}

/// Minimal user response
#[derive(Debug, Clone, Deserialize)]
pub struct GhUser {
    pub login: String,
}

/// Merge method for pull requests
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

/// GitHub operations client using reqwest
pub struct GitHubOperations {
    client: Client,
    token: String,
}

impl GitHubOperations {
    /// Create a new GitHubOperations client with a personal access token
    pub fn new(token: &str) -> Result<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| GitHubError::GitHubApi(format!("Failed to create HTTP client: {e}")))?;

        Ok(Self {
            client,
            token: token.to_string(),
        })
    }

    /// Create from environment variable GITHUB_TOKEN
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("GITHUB_TOKEN").map_err(|_| {
            GitHubError::GitHubApi("GITHUB_TOKEN environment variable not set".into())
        })?;
        Self::new(&token)
    }

    fn auth_headers(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        request
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
    }

    /// Create a pull request
    pub async fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
    ) -> Result<GhPullRequest> {
        info!("Creating PR: {} -> {} in {}/{}", head, base, owner, repo);

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls");
        let resp = self
            .auth_headers(self.client.post(&url))
            .json(&serde_json::json!({
                "title": title,
                "body": body,
                "head": head,
                "base": base
            }))
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Create PR failed ({status}): {text}"
            )));
        }

        let pr: GhPullRequest = resp
            .json()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Failed to parse response: {e}")))?;

        info!(
            "Created PR #{}: {}",
            pr.number,
            pr.html_url.as_deref().unwrap_or("")
        );
        Ok(pr)
    }

    /// List pull requests
    pub async fn list_prs(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
    ) -> Result<Vec<GhPullRequest>> {
        debug!("Listing PRs for {}/{}", owner, repo);

        let state_str = state.unwrap_or("open");
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls?state={state_str}");
        let resp = self
            .auth_headers(self.client.get(&url))
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "List PRs failed ({status}): {text}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Failed to parse response: {e}")))
    }

    /// Merge a pull request
    pub async fn merge_pr(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        method: MergeMethod,
    ) -> Result<()> {
        info!(
            "Merging PR #{} in {}/{} with method {:?}",
            number, owner, repo, method
        );

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls/{number}/merge");
        let method_str = match method {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        };

        let resp = self
            .auth_headers(self.client.put(&url))
            .json(&serde_json::json!({ "merge_method": method_str }))
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Merge PR failed ({status}): {text}"
            )));
        }

        info!("Merged PR #{}", number);
        Ok(())
    }

    /// Create an issue
    pub async fn create_issue(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
    ) -> Result<GhIssue> {
        info!("Creating issue in {}/{}: {}", owner, repo, title);

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues");
        let resp = self
            .auth_headers(self.client.post(&url))
            .json(&serde_json::json!({
                "title": title,
                "body": body
            }))
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Create issue failed ({status}): {text}"
            )));
        }

        let issue: GhIssue = resp
            .json()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Failed to parse response: {e}")))?;

        info!("Created issue #{}: {}", issue.number, issue.html_url);
        Ok(issue)
    }

    /// List issues
    pub async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        state: Option<&str>,
    ) -> Result<Vec<GhIssue>> {
        debug!("Listing issues for {}/{}", owner, repo);

        let state_str = state.unwrap_or("open");
        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues?state={state_str}");
        let resp = self
            .auth_headers(self.client.get(&url))
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "List issues failed ({status}): {text}"
            )));
        }

        resp.json()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Failed to parse response: {e}")))
    }

    /// Create a release
    pub async fn create_release(
        &self,
        owner: &str,
        repo: &str,
        tag: &str,
        name: &str,
        body: &str,
    ) -> Result<GhRelease> {
        info!("Creating release {} in {}/{}", tag, owner, repo);

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/releases");
        let resp = self
            .auth_headers(self.client.post(&url))
            .json(&serde_json::json!({
                "tag_name": tag,
                "name": name,
                "body": body
            }))
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Create release failed ({status}): {text}"
            )));
        }

        let release: GhRelease = resp
            .json()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Failed to parse response: {e}")))?;

        info!("Created release: {}", release.html_url);
        Ok(release)
    }

    /// Add a comment to an issue or PR
    pub async fn add_comment(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<()> {
        debug!("Adding comment to {}/{}#{}", owner, repo, number);

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{number}/comments");
        let resp = self
            .auth_headers(self.client.post(&url))
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Add comment failed ({status}): {text}"
            )));
        }

        Ok(())
    }

    /// Add labels to an issue or PR
    pub async fn add_labels(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        labels: &[String],
    ) -> Result<()> {
        debug!(
            "Adding labels to {}/{}#{}: {:?}",
            owner, repo, number, labels
        );

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{number}/labels");
        let resp = self
            .auth_headers(self.client.post(&url))
            .json(&serde_json::json!({ "labels": labels }))
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Add labels failed ({status}): {text}"
            )));
        }

        Ok(())
    }

    /// Assign users to an issue or PR
    pub async fn add_assignees(
        &self,
        owner: &str,
        repo: &str,
        number: u64,
        assignees: &[String],
    ) -> Result<()> {
        debug!(
            "Adding assignees to {}/{}#{}: {:?}",
            owner, repo, number, assignees
        );

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{number}/assignees");
        let resp = self
            .auth_headers(self.client.post(&url))
            .json(&serde_json::json!({ "assignees": assignees }))
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Add assignees failed ({status}): {text}"
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_method_serialization() {
        let merge = MergeMethod::Squash;
        let json = serde_json::to_string(&merge).unwrap();
        assert_eq!(json, "\"squash\"");
    }
}
