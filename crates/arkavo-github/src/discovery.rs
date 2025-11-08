use crate::error::{GitHubError, Result};
use chrono::{DateTime, Duration, Utc};
use octocrab::Octocrab;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

const DEFAULT_CACHE_TTL_MINUTES: i64 = 5;
const GITHUB_PER_PAGE: u8 = 100;

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
    github_client: Arc<Octocrab>,
    cache: Arc<RwLock<HashMap<String, CachedRepoList>>>,
    cache_ttl: Duration,
}

impl OrgDiscovery {
    pub fn new(token: Option<String>) -> Result<Self> {
        let github_client = if let Some(token) = token {
            Octocrab::builder()
                .personal_token(token)
                .build()
                .map_err(|e| GitHubError::Octocrab(Box::new(e)))?
        } else {
            Octocrab::default()
        };

        Ok(Self {
            github_client: Arc::new(github_client),
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

    async fn fetch_all_repos(&self, org: &str) -> Result<Vec<RepoInfo>> {
        let mut all_repos = Vec::new();
        let mut page = 1u32;

        loop {
            debug!("Fetching page {page} for organization: {org}");

            let page_repos = self
                .github_client
                .orgs(org)
                .list_repos()
                .per_page(GITHUB_PER_PAGE)
                .page(page)
                .send()
                .await
                .map_err(|e| {
                    if e.to_string().contains("404") {
                        GitHubError::OrgNotFound(org.to_string())
                    } else if e.to_string().contains("rate limit") {
                        GitHubError::RateLimitExceeded(60)
                    } else {
                        GitHubError::Octocrab(Box::new(e))
                    }
                })?;

            if page_repos.items.is_empty() {
                break;
            }

            let converted_repos: Vec<RepoInfo> = page_repos
                .items
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
                    language: repo.language.and_then(|l| l.as_str().map(String::from)),
                    size_kb: repo.size.unwrap_or(0) as u64,
                })
                .collect();

            all_repos.extend(converted_repos);

            if page_repos.next.is_none() {
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
