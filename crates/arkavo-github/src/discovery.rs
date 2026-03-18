use crate::error::{GitHubError, Result};
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

const DEFAULT_CACHE_TTL_MINUTES: i64 = 5;
const GITHUB_PER_PAGE: u32 = 100;
const GITHUB_API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("Arkavo-GitHub/", env!("CARGO_PKG_VERSION"));

/// Minimal repo response from GitHub API (only fields we use)
#[derive(Debug, Clone, Deserialize)]
struct GhRepo {
    full_name: Option<String>,
    name: String,
    archived: Option<bool>,
    private: Option<bool>,
    default_branch: Option<String>,
    language: Option<String>,
    size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoInfo {
    pub full_name: String,
    pub owner: String,
    pub name: String,
    pub is_archived: bool,
    pub is_private: bool,
    pub default_branch: String,
    pub language: Option<String>,
    pub size_kb: u64,
}

#[derive(Debug, Clone)]
struct CachedRepoList {
    repos: Vec<RepoInfo>,
    cached_at: DateTime<Utc>,
}

pub struct OrgDiscovery {
    client: Client,
    token: Option<String>,
    cache: Arc<RwLock<HashMap<String, CachedRepoList>>>,
    cache_ttl: Duration,
}

impl OrgDiscovery {
    pub fn new(token: Option<String>) -> Result<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| GitHubError::GitHubApi(format!("Failed to create HTTP client: {e}")))?;

        Ok(Self {
            client,
            token,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_ttl: Duration::minutes(DEFAULT_CACHE_TTL_MINUTES),
        })
    }

    pub fn with_cache_ttl(mut self, ttl_minutes: i64) -> Self {
        self.cache_ttl = Duration::minutes(ttl_minutes);
        self
    }

    pub async fn discover_repos(&self, org: &str) -> Result<Vec<RepoInfo>> {
        if let Some(cached) = self.get_cached_repos(org).await {
            debug!("Using cached repos for organization: {org}");
            return Ok(cached);
        }

        info!("Discovering repositories for organization: {org}");
        let repos = self.fetch_all_repos(org).await?;

        if repos.is_empty() {
            warn!("No repositories found for organization: {org}");
            return Err(GitHubError::NoRepositories(org.to_string()));
        }

        self.update_cache(org, repos.clone()).await;
        info!("Discovered {} repositories for {org}", repos.len());

        Ok(repos)
    }

    pub async fn refresh_cache(&self, org: &str) -> Result<()> {
        info!("Refreshing cache for organization: {org}");
        let repos = self.fetch_all_repos(org).await?;
        self.update_cache(org, repos).await;
        Ok(())
    }

    pub async fn is_cache_valid(&self, org: &str) -> bool {
        let cache = self.cache.read().await;
        if let Some(cached) = cache.get(org) {
            let age = Utc::now() - cached.cached_at;
            age < self.cache_ttl
        } else {
            false
        }
    }

    async fn get_cached_repos(&self, org: &str) -> Option<Vec<RepoInfo>> {
        let cached = self.cache.read().await.get(org).cloned()?;
        let age = Utc::now() - cached.cached_at;
        if age < self.cache_ttl {
            Some(cached.repos)
        } else {
            None
        }
    }

    async fn update_cache(&self, org: &str, repos: Vec<RepoInfo>) {
        let mut cache = self.cache.write().await;
        cache.insert(
            org.to_string(),
            CachedRepoList {
                repos,
                cached_at: Utc::now(),
            },
        );
    }

    fn auth_headers(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = request.header("Accept", "application/vnd.github+json");
        if let Some(ref token) = self.token {
            req.header("Authorization", format!("Bearer {token}"))
        } else {
            req
        }
    }

    async fn fetch_all_repos(&self, org: &str) -> Result<Vec<RepoInfo>> {
        let mut all_repos = Vec::new();
        let mut page = 1u32;

        loop {
            debug!("Fetching page {page} for organization: {org}");

            let url = format!(
                "{GITHUB_API_BASE}/orgs/{org}/repos?per_page={GITHUB_PER_PAGE}&page={page}"
            );

            let resp = self
                .auth_headers(self.client.get(&url))
                .send()
                .await
                .map_err(|e| GitHubError::GitHubApi(format!("Request failed: {e}")))?;

            let status = resp.status();
            if status == reqwest::StatusCode::NOT_FOUND {
                return Err(GitHubError::OrgNotFound(org.to_string()));
            }
            if status == reqwest::StatusCode::FORBIDDEN {
                return Err(GitHubError::RateLimitExceeded(60));
            }
            if !status.is_success() {
                let text = resp.text().await.unwrap_or_default();
                return Err(GitHubError::GitHubApi(format!(
                    "List repos failed ({status}): {text}"
                )));
            }

            let page_repos: Vec<GhRepo> = resp
                .json()
                .await
                .map_err(|e| GitHubError::GitHubApi(format!("Failed to parse response: {e}")))?;

            if page_repos.is_empty() {
                break;
            }

            let converted_repos: Vec<RepoInfo> = page_repos
                .into_iter()
                .map(|repo| RepoInfo {
                    full_name: repo
                        .full_name
                        .unwrap_or_else(|| format!("{org}/{}", repo.name)),
                    owner: org.to_string(),
                    name: repo.name,
                    is_archived: repo.archived.unwrap_or(false),
                    is_private: repo.private.unwrap_or(false),
                    default_branch: repo.default_branch.unwrap_or_else(|| "main".to_string()),
                    language: repo.language,
                    size_kb: repo.size.unwrap_or(0),
                })
                .collect();

            let got_fewer_than_page = converted_repos.len() < GITHUB_PER_PAGE as usize;
            all_repos.extend(converted_repos);

            if got_fewer_than_page {
                break;
            }

            page += 1;
        }

        Ok(all_repos)
    }

    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_discover_repos_real() {
        let token = std::env::var("GITHUB_TOKEN").ok();
        let discovery = OrgDiscovery::new(token).unwrap();

        let test_org = std::env::var("TEST_GITHUB_ORG").unwrap_or_else(|_| "rust-lang".to_string());

        let repos = discovery.discover_repos(&test_org).await;

        match repos {
            Ok(repos) => {
                assert!(!repos.is_empty(), "{test_org} should have repositories");
                println!("Found {} repositories in {test_org}", repos.len());
                for repo in repos.iter().take(5) {
                    println!("  - {}", repo.full_name);
                }
            }
            Err(e) => {
                println!("Error (expected if no token): {e}");
            }
        }
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let token = std::env::var("GITHUB_TOKEN").ok();
        let discovery = OrgDiscovery::new(token).unwrap().with_cache_ttl(0);

        assert!(!discovery.is_cache_valid("test-org").await);
    }
}
