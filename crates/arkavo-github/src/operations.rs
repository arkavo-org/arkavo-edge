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

/// Optional conclusion and output fields for Check Run create/update calls.
#[derive(Debug, Clone, Default)]
pub struct CheckRunDetails<'a> {
    /// `success`, `failure`, `neutral`, `cancelled`, `skipped`, `timed_out`, or `action_required`.
    pub conclusion: Option<&'a str>,
    /// Short title for the output section.
    pub output_title: Option<&'a str>,
    /// Markdown summary for the output section.
    pub output_summary: Option<&'a str>,
}

/// GitHub operations client using reqwest
pub struct GitHubOperations {
    client: Client,
    token: String,
    base_url: String,
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
            base_url: GITHUB_API_BASE.to_string(),
        })
    }

    /// Construct with a custom API base (used by tests against a mock server).
    pub fn with_base_url(token: &str, base_url: &str) -> Result<Self> {
        let mut ops = Self::new(token)?;
        ops.base_url = base_url.to_string();
        Ok(ops)
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

    /// Create a Check Run on `head_sha`. Returns the new check run id.
    pub async fn create_check_run(
        &self,
        owner: &str,
        repo: &str,
        name: &str,
        head_sha: &str,
        status: &str,
        details: CheckRunDetails<'_>,
    ) -> Result<u64> {
        let base = &self.base_url;
        let url = format!("{base}/repos/{owner}/{repo}/check-runs");
        let body = check_run_create_body(
            name,
            head_sha,
            status,
            details.conclusion,
            details.output_title,
            details.output_summary,
        );
        let resp = self
            .auth_headers(self.client.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;
        let status_code = resp.status();
        if !status_code.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Create check run failed ({status_code}): {text}"
            )));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Bad check-run response: {e}")))?;
        json["id"]
            .as_u64()
            .ok_or_else(|| GitHubError::GitHubApi("check-run response missing id".into()))
    }

    /// Update an existing Check Run (set status/conclusion/output).
    pub async fn update_check_run(
        &self,
        owner: &str,
        repo: &str,
        check_run_id: u64,
        status: &str,
        details: CheckRunDetails<'_>,
    ) -> Result<()> {
        let base = &self.base_url;
        let url = format!("{base}/repos/{owner}/{repo}/check-runs/{check_run_id}");
        let body = check_run_update_body(
            status,
            details.conclusion,
            details.output_title,
            details.output_summary,
        );
        let resp = self
            .auth_headers(self.client.patch(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;
        let status_code = resp.status();
        if !status_code.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Update check run failed ({status_code}): {text}"
            )));
        }
        Ok(())
    }
}

fn check_run_create_body(
    name: &str,
    head_sha: &str,
    status: &str,
    conclusion: Option<&str>,
    output_title: Option<&str>,
    output_summary: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "name": name,
        "head_sha": head_sha,
        "status": status,
    });
    if let Some(c) = conclusion {
        body["conclusion"] = serde_json::json!(c);
    }
    if let (Some(t), Some(s)) = (output_title, output_summary) {
        body["output"] = serde_json::json!({ "title": t, "summary": s });
    }
    body
}

fn check_run_update_body(
    status: &str,
    conclusion: Option<&str>,
    output_title: Option<&str>,
    output_summary: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({ "status": status });
    if let Some(c) = conclusion {
        body["conclusion"] = serde_json::json!(c);
    }
    if let (Some(t), Some(s)) = (output_title, output_summary) {
        body["output"] = serde_json::json!({ "title": t, "summary": s });
    }
    body
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

    #[test]
    fn check_run_create_body_required_fields() {
        let body = check_run_create_body("eval", "abc123", "in_progress", None, None, None);
        assert_eq!(body["name"], "eval");
        assert_eq!(body["head_sha"], "abc123");
        assert_eq!(body["status"], "in_progress");
        assert!(body.get("conclusion").is_none() || body["conclusion"].is_null());
        assert!(body.get("output").is_none() || body["output"].is_null());
    }

    #[test]
    fn check_run_create_body_with_conclusion_and_output() {
        let body = check_run_create_body(
            "eval",
            "abc123",
            "completed",
            Some("success"),
            Some("Eval passed"),
            Some("ok"),
        );
        assert_eq!(body["conclusion"], "success");
        assert_eq!(body["output"]["title"], "Eval passed");
        assert_eq!(body["output"]["summary"], "ok");
    }

    #[test]
    fn check_run_update_body_required_field() {
        let body = check_run_update_body("queued", None, None, None);
        assert_eq!(body["status"], "queued");
        assert!(body.get("conclusion").is_none() || body["conclusion"].is_null());
        assert!(body.get("output").is_none() || body["output"].is_null());
    }

    #[test]
    fn check_run_update_body_with_conclusion_and_output() {
        let body = check_run_update_body(
            "completed",
            Some("failure"),
            Some("Regression"),
            Some("similarity 0.5 < 0.87"),
        );
        assert_eq!(body["conclusion"], "failure");
        assert_eq!(body["output"]["title"], "Regression");
        assert_eq!(body["output"]["summary"], "similarity 0.5 < 0.87");
    }
}
