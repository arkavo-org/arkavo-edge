//! `github_pr_watch` MCP tool — stateless PR-discovery for the swarm dispatcher.
//!
//! Lists a repo's open PRs with each PR's head SHA + last-update time, newest first.
//! An optional `since` timestamp filters to only PRs updated strictly after it,
//! so the caller (the swarm) can own the cursor without server-side state.

use crate::github_api::{GITHUB_API_BASE, GhPullRequest, get_github_client, github_request};
use crate::server::{Tool, ToolSchema};
use crate::{Result, ToolError};
use async_trait::async_trait;
use serde::Serialize;

/// One PR as surfaced to the swarm: identity + head SHA + last-update time.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PrSummary {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) author: String,
    pub(crate) head_sha: String,
    pub(crate) updated_at: String,
}

/// Pure: map open PRs to summaries, keeping only those updated strictly after
/// `since` (lexicographic compare is correct for ISO-8601 UTC `Z` timestamps).
/// PRs missing a head SHA or `updated_at` are dropped (can't be tracked).
pub(crate) fn select_updated_prs(prs: Vec<GhPullRequest>, since: Option<&str>) -> Vec<PrSummary> {
    prs.into_iter()
        .filter_map(|p| {
            let head_sha = p.head.as_ref().map(|h| h.sha.clone())?;
            let updated_at = p.updated_at.clone()?;
            if let Some(since) = since
                && updated_at.as_str() <= since
            {
                return None;
            }
            Some(PrSummary {
                number: p.number,
                title: p.title.unwrap_or_default(),
                url: p.html_url.unwrap_or_default(),
                author: p.user.map(|u| u.login).unwrap_or_default(),
                head_sha,
                updated_at,
            })
        })
        .collect()
}

pub struct GitHubPrWatchTool {
    schema: ToolSchema,
}

impl GitHubPrWatchTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "github_pr_watch".to_string(),
                aliases: None,
                description: "List a repository's OPEN pull requests with each PR's head SHA and last-update time, newest first. Pass `since` (an ISO-8601 UTC timestamp from a prior call) to get only PRs updated after it — use this to discover new or changed PRs to review. Args: owner, repo, since (optional).".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner / org login." },
                        "repo": { "type": "string", "description": "Repository name." },
                        "since": { "type": "string", "description": "Optional ISO-8601 UTC timestamp; return only PRs updated after this." }
                    },
                    "required": ["owner", "repo"]
                }),
            },
        }
    }
}

impl Default for GitHubPrWatchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GitHubPrWatchTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, params: serde_json::Value) -> Result<serde_json::Value> {
        let owner = params
            .get("owner")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("github_pr_watch: missing 'owner'".into()))?;
        let repo = params
            .get("repo")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParams("github_pr_watch: missing 'repo'".into()))?;
        let since = params.get("since").and_then(|v| v.as_str());

        let gh = get_github_client().await?;
        let url = format!(
            "{GITHUB_API_BASE}/repos/{owner}/{repo}/pulls?state=open&sort=updated&direction=desc&per_page=100"
        );
        let resp = github_request(gh, gh.client.get(&url), "github_pr_watch").await?;
        let prs: Vec<GhPullRequest> = resp
            .json()
            .await
            .map_err(|e| ToolError::Mcp(format!("github_pr_watch: parse PRs failed: {e}")))?;

        let selected = select_updated_prs(prs, since);
        Ok(serde_json::json!({
            "owner": owner,
            "repo": repo,
            "count": selected.len(),
            "pull_requests": selected,
        }))
    }
}

#[cfg(test)]
mod pr_watch_tests {
    use super::*;
    use crate::github_api::{GhHead, GhPullRequest, GhUser};

    fn pr(number: u64, sha: &str, updated: &str) -> GhPullRequest {
        GhPullRequest {
            number,
            title: Some(format!("PR {number}")),
            html_url: Some(format!("https://example/pr/{number}")),
            state: Some("open".into()),
            user: Some(GhUser {
                login: "alice".into(),
            }),
            head: Some(GhHead { sha: sha.into() }),
            updated_at: Some(updated.into()),
        }
    }

    #[test]
    fn no_since_returns_all() {
        let prs = vec![
            pr(1, "aaa", "2026-06-18T10:00:00Z"),
            pr(2, "bbb", "2026-06-18T12:00:00Z"),
        ];
        let out = select_updated_prs(prs, None);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].head_sha, "aaa");
    }

    #[test]
    fn since_excludes_unchanged_and_includes_newer() {
        let prs = vec![
            pr(1, "aaa", "2026-06-18T10:00:00Z"), // unchanged: == since
            pr(2, "bbb", "2026-06-18T13:00:00Z"), // changed: newer than since
        ];
        let out = select_updated_prs(prs, Some("2026-06-18T10:00:00Z"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].number, 2);
        assert_eq!(out[0].head_sha, "bbb");
    }

    #[test]
    fn missing_updated_at_is_excluded_when_since_set() {
        let mut p = pr(3, "ccc", "");
        p.updated_at = None;
        let out = select_updated_prs(vec![p], Some("2026-06-18T10:00:00Z"));
        assert!(out.is_empty());
    }
}
