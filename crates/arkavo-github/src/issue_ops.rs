use crate::app_auth::GitHubApp;
use crate::error::{GitHubError, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tracing::{debug, info};

const GITHUB_API_BASE: &str = "https://api.github.com";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentRequest {
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelUpdate {
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignees: Option<Vec<String>>,
}

pub struct IssueOperations {
    github_app: Arc<GitHubApp>,
    client: Client,
}

impl IssueOperations {
    pub fn new(github_app: Arc<GitHubApp>) -> Self {
        let client = Client::builder()
            .user_agent("arkavo-edge")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { github_app, client }
    }

    async fn get_token(&self, owner: &str, repo: &str) -> Result<String> {
        self.github_app.get_token(owner, repo).await
    }

    pub async fn post_comment(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<()> {
        let token = self.get_token(owner, repo).await?;

        info!(owner, repo, issue_number, "Posting comment to GitHub issue");

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{issue_number}/comments");

        let request = CommentRequest {
            body: body.to_string(),
        };

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Failed to post comment: {status} - {error_text}"
            )));
        }

        debug!("Comment posted successfully");
        Ok(())
    }

    pub async fn add_labels(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        labels: Vec<String>,
    ) -> Result<()> {
        let token = self.get_token(owner, repo).await?;

        info!(
            owner,
            repo,
            issue_number,
            labels = ?labels,
            "Adding labels to GitHub issue"
        );

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{issue_number}/labels");

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&json!({"labels": labels}))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Failed to add labels: {status} - {error_text}"
            )));
        }

        debug!("Labels added successfully");
        Ok(())
    }

    pub async fn remove_label(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()> {
        let token = self.get_token(owner, repo).await?;

        info!(
            owner,
            repo, issue_number, label, "Removing label from GitHub issue"
        );

        let url =
            format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{issue_number}/labels/{label}");

        let response = self
            .client
            .delete(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;

        if !response.status().is_success() && response.status().as_u16() != 404 {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Failed to remove label: {status} - {error_text}"
            )));
        }

        debug!("Label removed successfully");
        Ok(())
    }

    pub async fn update_issue(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        update: IssueUpdate,
    ) -> Result<()> {
        let token = self.get_token(owner, repo).await?;

        info!(owner, repo, issue_number, "Updating GitHub issue");

        let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/issues/{issue_number}");

        let response = self
            .client
            .patch(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&update)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Failed to update issue: {status} - {error_text}"
            )));
        }

        debug!("Issue updated successfully");
        Ok(())
    }

    pub async fn close_issue(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        comment: Option<&str>,
    ) -> Result<()> {
        if let Some(body) = comment {
            self.post_comment(owner, repo, issue_number, body).await?;
        }

        let update = IssueUpdate {
            state: Some("closed".to_string()),
            labels: None,
            assignees: None,
        };

        self.update_issue(owner, repo, issue_number, update).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comment_request_serialization() {
        let request = CommentRequest {
            body: "Test comment".to_string(),
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("Test comment"));
    }

    #[test]
    fn test_issue_update_serialization() {
        let update = IssueUpdate {
            state: Some("closed".to_string()),
            labels: Some(vec!["fixed".to_string()]),
            assignees: None,
        };
        let json = serde_json::to_string(&update).unwrap();
        assert!(json.contains("closed"));
        assert!(json.contains("fixed"));
        assert!(!json.contains("assignees"));
    }
}
