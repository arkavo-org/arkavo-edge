//! GitHub API operations for PRs, Issues, and Releases
//!
//! Provides direct API access via octocrab, replacing the gh CLI dependency.

use crate::error::{GitHubError, Result};
use octocrab::Octocrab;
use octocrab::models::issues::Issue;
use octocrab::models::pulls::PullRequest;
use octocrab::models::repos::Release;
use octocrab::params::State;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

/// Merge method for pull requests
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

/// GitHub operations client using octocrab
pub struct GitHubOperations {
    client: Arc<Octocrab>,
}

impl GitHubOperations {
    /// Create a new GitHubOperations client with a personal access token
    pub fn new(token: &str) -> Result<Self> {
        let client = Octocrab::builder()
            .personal_token(token.to_string())
            .build()
            .map_err(|e| GitHubError::Octocrab(Box::new(e)))?;

        Ok(Self {
            client: Arc::new(client),
        })
    }

    /// Create from environment variable GITHUB_TOKEN
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("GITHUB_TOKEN").map_err(|_| {
            GitHubError::GitHubApi("GITHUB_TOKEN environment variable not set".into())
        })?;
        Self::new(&token)
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
    ) -> Result<PullRequest> {
        info!("Creating PR: {} -> {} in {}/{}", head, base, owner, repo);

        let pr = self
            .client
            .pulls(owner, repo)
            .create(title, head, base)
            .body(body)
            .send()
            .await
            .map_err(|e| GitHubError::Octocrab(Box::new(e)))?;

        info!(
            "Created PR #{}: {}",
            pr.number,
            pr.html_url.as_ref().map(|u| u.as_str()).unwrap_or("")
        );
        Ok(pr)
    }

    /// List pull requests
    pub async fn list_prs(
        &self,
        owner: &str,
        repo: &str,
        state: Option<State>,
    ) -> Result<Vec<PullRequest>> {
        debug!("Listing PRs for {}/{}", owner, repo);

        let pulls_handler = self.client.pulls(owner, repo);
        let page = match state {
            Some(s) => pulls_handler.list().state(s).send().await,
            None => pulls_handler.list().send().await,
        }
        .map_err(|e| GitHubError::Octocrab(Box::new(e)))?;

        Ok(page.items)
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

        let merge_method = match method {
            MergeMethod::Merge => octocrab::params::pulls::MergeMethod::Merge,
            MergeMethod::Squash => octocrab::params::pulls::MergeMethod::Squash,
            MergeMethod::Rebase => octocrab::params::pulls::MergeMethod::Rebase,
        };

        self.client
            .pulls(owner, repo)
            .merge(number)
            .method(merge_method)
            .send()
            .await
            .map_err(|e| GitHubError::Octocrab(Box::new(e)))?;

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
    ) -> Result<Issue> {
        info!("Creating issue in {}/{}: {}", owner, repo, title);

        let issue = self
            .client
            .issues(owner, repo)
            .create(title)
            .body(body)
            .send()
            .await
            .map_err(|e| GitHubError::Octocrab(Box::new(e)))?;

        info!("Created issue #{}: {}", issue.number, issue.html_url);
        Ok(issue)
    }

    /// List issues
    pub async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        state: Option<State>,
    ) -> Result<Vec<Issue>> {
        debug!("Listing issues for {}/{}", owner, repo);

        let issues_handler = self.client.issues(owner, repo);
        let page = match state {
            Some(s) => issues_handler.list().state(s).send().await,
            None => issues_handler.list().send().await,
        }
        .map_err(|e| GitHubError::Octocrab(Box::new(e)))?;

        Ok(page.items)
    }

    /// Create a release
    pub async fn create_release(
        &self,
        owner: &str,
        repo: &str,
        tag: &str,
        name: &str,
        body: &str,
    ) -> Result<Release> {
        info!("Creating release {} in {}/{}", tag, owner, repo);

        let release = self
            .client
            .repos(owner, repo)
            .releases()
            .create(tag)
            .name(name)
            .body(body)
            .send()
            .await
            .map_err(|e| GitHubError::Octocrab(Box::new(e)))?;

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

        self.client
            .issues(owner, repo)
            .create_comment(number, body)
            .await
            .map_err(|e| GitHubError::Octocrab(Box::new(e)))?;

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

        self.client
            .issues(owner, repo)
            .add_labels(number, labels)
            .await
            .map_err(|e| GitHubError::Octocrab(Box::new(e)))?;

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

        let assignees_refs: Vec<&str> = assignees.iter().map(|s| s.as_str()).collect();
        self.client
            .issues(owner, repo)
            .add_assignees(number, &assignees_refs)
            .await
            .map_err(|e| GitHubError::Octocrab(Box::new(e)))?;

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
