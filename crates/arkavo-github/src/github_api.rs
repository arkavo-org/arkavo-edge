use crate::error::{GitHubError, Result};
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::warn;

const GITHUB_API_TIMEOUT_SECS: u64 = 30;
const MAX_RATE_LIMIT_RETRIES: usize = 3;
const USER_AGENT: &str = "Arkavo-Orchestrator/0.38";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub user: GitHubUser,
    pub labels: Vec<GitHubLabel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubUser {
    pub login: String,
    pub id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubLabel {
    pub name: String,
}

fn create_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(GITHUB_API_TIMEOUT_SECS))
        .build()
        .map_err(|e| GitHubError::GitHubApi(format!("Failed to create HTTP client: {e}")))
}

pub async fn fetch_new_issues(
    org: &str,
    repo_name: &str,
    token: &str,
    since: DateTime<Utc>,
    label_filter: Option<&str>,
) -> Result<Vec<GitHubIssue>> {
    fetch_new_issues_impl(
        org.to_string(),
        repo_name.to_string(),
        token.to_string(),
        since,
        label_filter.map(|s| s.to_string()),
        0,
    )
    .await
}

fn fetch_new_issues_impl(
    org: String,
    repo_name: String,
    token: String,
    since: DateTime<Utc>,
    label_filter: Option<String>,
    retry_count: usize,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<GitHubIssue>>> + Send>> {
    Box::pin(async move {
        let client = create_client()?;

        let mut url = format!(
            "https://api.github.com/repos/{org}/{repo_name}/issues?state=open&since={}",
            since.to_rfc3339()
        );

        if let Some(labels) = &label_filter {
            url = format!("{url}&labels={labels}");
        }

        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

        match response.status() {
            StatusCode::OK => {
                let issues: Vec<GitHubIssue> = response.json().await.map_err(|e| {
                    GitHubError::GitHubApi(format!("Failed to parse response: {e}"))
                })?;
                Ok(issues)
            }
            StatusCode::FORBIDDEN => {
                if retry_count >= MAX_RATE_LIMIT_RETRIES {
                    return Err(GitHubError::RateLimitExceeded(60));
                }

                if let Some(reset) = response.headers().get("x-ratelimit-reset")
                    && let Ok(reset_str) = reset.to_str()
                    && let Ok(reset_time) = reset_str.parse::<i64>()
                {
                    let now = Utc::now().timestamp();
                    let wait_seconds = (reset_time - now).max(0) as u64;
                    warn!(
                        "Rate limited on '{org}/{repo_name}'. Waiting {wait_seconds} seconds (attempt {}/{})",
                        retry_count + 1,
                        MAX_RATE_LIMIT_RETRIES
                    );
                    tokio::time::sleep(Duration::from_secs(wait_seconds + 1)).await;
                    return fetch_new_issues_impl(
                        org,
                        repo_name,
                        token,
                        since,
                        label_filter,
                        retry_count + 1,
                    )
                    .await;
                }
                Err(GitHubError::RateLimitExceeded(60))
            }
            status => {
                let error_text = response
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown error".to_string());
                Err(GitHubError::GitHubApi(format!(
                    "GitHub API error for '{org}/{repo_name}': {status} - {error_text}"
                )))
            }
        }
    })
}
