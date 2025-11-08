use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use tokio::time::Duration;
use tracing::warn;

use super::constants::{GITHUB_API_TIMEOUT_SECS, MAX_RATE_LIMIT_RETRIES, USER_AGENT};
use super::types::{GitHubIssue, GitHubRepository};

pub(super) fn parse_repo(repo: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        return Err(anyhow!(
            "Invalid repository format. Expected 'owner/repo', got '{repo}'"
        ));
    }

    let owner = parts[0].trim();
    let repo_name = parts[1].trim();

    if owner.is_empty() || repo_name.is_empty() {
        return Err(anyhow!("Owner and repository name cannot be empty"));
    }

    Ok((owner.to_string(), repo_name.to_string()))
}

fn create_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(GITHUB_API_TIMEOUT_SECS))
        .build()
        .map_err(|e| anyhow!("Failed to create HTTP client: {e}"))
}

pub(super) async fn fetch_issue(repo: &str, token: &str, issue_number: u64) -> Result<GitHubIssue> {
    let (owner, repo_name) = parse_repo(repo)?;
    let client = create_client()?;

    let url = format!("https://api.github.com/repos/{owner}/{repo_name}/issues/{issue_number}");

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub API error for issue #{} in {}: {} - {}",
            issue_number,
            repo,
            response.status(),
            response.text().await?
        ));
    }

    Ok(response.json().await?)
}

pub(super) async fn fetch_repository(repo: &str, token: &str) -> Result<GitHubRepository> {
    let (owner, repo_name) = parse_repo(repo)?;
    let client = create_client()?;

    let url = format!("https://api.github.com/repos/{owner}/{repo_name}");

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub API error for repository {}: {} - {}",
            repo,
            response.status(),
            response.text().await?
        ));
    }

    Ok(response.json().await?)
}

pub(super) async fn fetch_new_issues(
    repo: &str,
    token: &str,
    since: &DateTime<Utc>,
    label_filter: Option<&str>,
) -> Result<Vec<GitHubIssue>> {
    fetch_new_issues_impl(
        repo.to_string(),
        token.to_string(),
        *since,
        label_filter.map(|s| s.to_string()),
        0,
    )
    .await
}

fn fetch_new_issues_impl(
    repo: String,
    token: String,
    since: DateTime<Utc>,
    label_filter: Option<String>,
    retry_count: usize,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<GitHubIssue>>> + Send>> {
    Box::pin(async move {
        let (owner, repo_name) = parse_repo(&repo)?;
        let client = create_client()?;

        let mut url = format!(
            "https://api.github.com/repos/{}/{}/issues?state=open&since={}",
            owner,
            repo_name,
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
            .await?;

        match response.status() {
            StatusCode::OK => {
                let issues: Vec<GitHubIssue> = response.json().await?;
                Ok(issues)
            }
            StatusCode::FORBIDDEN => {
                if retry_count >= MAX_RATE_LIMIT_RETRIES {
                    return Err(anyhow!(
                        "Exceeded maximum retry attempts ({MAX_RATE_LIMIT_RETRIES}) for rate limiting on repository '{repo}'"
                    ));
                }

                if let Some(reset) = response.headers().get("x-ratelimit-reset")
                    && let Ok(reset_str) = reset.to_str()
                    && let Ok(reset_time) = reset_str.parse::<i64>()
                {
                    let now = Utc::now().timestamp();
                    let wait_seconds = (reset_time - now).max(0) as u64;
                    warn!(
                        "Rate limited on '{}'. Waiting {} seconds (attempt {}/{})",
                        repo,
                        wait_seconds,
                        retry_count + 1,
                        MAX_RATE_LIMIT_RETRIES
                    );
                    tokio::time::sleep(Duration::from_secs(wait_seconds + 1)).await;
                    return fetch_new_issues_impl(
                        repo,
                        token,
                        since,
                        label_filter,
                        retry_count + 1,
                    )
                    .await;
                }
                Err(anyhow!(
                    "GitHub API forbidden for '{}': {}",
                    repo,
                    response.text().await?
                ))
            }
            status => Err(anyhow!(
                "GitHub API error for '{}': {} - {}",
                repo,
                status,
                response.text().await?
            )),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repo_valid() {
        let result = parse_repo("owner/repo").unwrap();
        assert_eq!(result, ("owner".to_string(), "repo".to_string()));
    }

    #[test]
    fn test_parse_repo_with_whitespace() {
        let result = parse_repo("  owner / repo  ").unwrap();
        assert_eq!(result, ("owner".to_string(), "repo".to_string()));
    }

    #[test]
    fn test_parse_repo_invalid() {
        assert!(parse_repo("invalid").is_err());
        assert!(parse_repo("owner/repo/extra").is_err());
        assert!(parse_repo("/repo").is_err());
        assert!(parse_repo("owner/").is_err());
        assert!(parse_repo("").is_err());
        assert!(parse_repo("/").is_err());
    }

    #[test]
    fn test_parse_repo_empty_parts() {
        assert!(parse_repo("  /  ").is_err());
        assert!(parse_repo("  /repo").is_err());
        assert!(parse_repo("owner/  ").is_err());
    }
}
