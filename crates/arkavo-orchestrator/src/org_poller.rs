use crate::error::Result;
use crate::orchestrator::Orchestrator;
use arkavo_memory::{OrchestratorStateStore, RepoState, RepoStatus};
use arkavo_org_discovery::{OrgDiscovery, RepoFilter};
use chrono::Utc;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

const DEFAULT_MAX_CONCURRENT_REPOS: usize = 10;
const DEFAULT_DISCOVERY_INTERVAL_SECS: u64 = 300;
const REPO_ERROR_THRESHOLD: u32 = 5;

pub struct OrgPollerConfig {
    pub poll_interval: Duration,
    pub max_concurrent_repos: usize,
    pub discovery_interval: Duration,
    pub repo_include_pattern: Option<String>,
    pub repo_exclude_pattern: Option<String>,
    pub include_archived: bool,
    pub labels_filter: Option<String>,
}

impl Default for OrgPollerConfig {
    fn default() -> Self {
        Self {
            poll_interval: Duration::from_secs(300),
            max_concurrent_repos: DEFAULT_MAX_CONCURRENT_REPOS,
            discovery_interval: Duration::from_secs(DEFAULT_DISCOVERY_INTERVAL_SECS),
            repo_include_pattern: None,
            repo_exclude_pattern: None,
            include_archived: false,
            labels_filter: None,
        }
    }
}

pub struct OrgPoller {
    org_name: String,
    orchestrator: Arc<Orchestrator>,
    discovery: Arc<OrgDiscovery>,
    state_store: Arc<OrchestratorStateStore>,
    config: OrgPollerConfig,
    active_tasks: Arc<RwLock<Vec<JoinHandle<()>>>>,
    running: Arc<RwLock<bool>>,
}

impl OrgPoller {
    pub async fn new(
        org_name: String,
        github_token: Option<String>,
        orchestrator: Arc<Orchestrator>,
        state_db_path: PathBuf,
        config: OrgPollerConfig,
    ) -> Result<Self> {
        let discovery = Arc::new(OrgDiscovery::new(github_token.clone())?);
        let state_store = Arc::new(OrchestratorStateStore::new(&state_db_path).await?);

        Ok(Self {
            org_name,
            orchestrator,
            discovery,
            state_store,
            config,
            active_tasks: Arc::new(RwLock::new(Vec::new())),
            running: Arc::new(RwLock::new(false)),
        })
    }

    pub async fn start(&self) -> Result<()> {
        let mut running = self.running.write().await;
        if *running {
            warn!("OrgPoller for {} is already running", self.org_name);
            return Ok(());
        }
        *running = true;
        drop(running);

        info!(
            "Starting OrgPoller for organization: {} (max {} concurrent repos)",
            self.org_name, self.config.max_concurrent_repos
        );

        let poller_clone = Arc::new(self.clone_self());
        let handle = tokio::spawn(async move {
            if let Err(e) = poller_clone.run_polling_loop().await {
                error!("OrgPoller error for {}: {}", poller_clone.org_name, e);
            }
        });

        self.active_tasks.write().await.push(handle);

        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        info!("Stopping OrgPoller for {}", self.org_name);
        *self.running.write().await = false;

        let mut tasks = self.active_tasks.write().await;
        for task in tasks.drain(..) {
            task.abort();
        }

        Ok(())
    }

    async fn run_polling_loop(&self) -> Result<()> {
        let mut last_discovery = std::time::Instant::now();

        loop {
            if !*self.running.read().await {
                info!("Polling loop stopped for {}", self.org_name);
                break;
            }

            if last_discovery.elapsed() > self.config.discovery_interval {
                if let Err(e) = self.discover_and_update_repos().await {
                    error!("Failed to discover repos for {}: {}", self.org_name, e);
                }
                last_discovery = std::time::Instant::now();
            }

            if let Err(e) = self.poll_all_repos().await {
                error!("Failed to poll repos for {}: {}", self.org_name, e);
            }

            tokio::time::sleep(self.config.poll_interval).await;
        }

        Ok(())
    }

    async fn discover_and_update_repos(&self) -> Result<()> {
        info!("Discovering repositories for {}", self.org_name);

        let repos = self.discovery.discover_repos(&self.org_name).await?;

        let mut filter = RepoFilter::new().with_archived(self.config.include_archived);

        if let Some(ref pattern) = self.config.repo_include_pattern {
            filter = filter.with_include_pattern(pattern)?;
        }

        if let Some(ref pattern) = self.config.repo_exclude_pattern {
            filter = filter.with_exclude_pattern(pattern)?;
        }

        let filtered_repos = filter.filter(repos);

        info!(
            "Discovered {} repositories for {} (after filtering)",
            filtered_repos.len(),
            self.org_name
        );

        for repo_info in filtered_repos {
            let existing_state = self
                .state_store
                .get_repo_state(&self.org_name, &repo_info.name)
                .await?;

            if existing_state.is_none() {
                let new_state = RepoState {
                    org: self.org_name.clone(),
                    repo_name: repo_info.name.clone(),
                    last_poll_at: Utc::now(),
                    issues_processed: 0,
                    issues_failed: 0,
                    last_error: None,
                    error_count: 0,
                    status: RepoStatus::Active,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };
                self.state_store.upsert_repo_state(&new_state).await?;
                debug!("Added new repository to state: {}", repo_info.full_name);
            }
        }

        Ok(())
    }

    async fn poll_all_repos(&self) -> Result<()> {
        let active_repos = self
            .state_store
            .list_repos(&self.org_name, Some(RepoStatus::Active))
            .await?;

        if active_repos.is_empty() {
            debug!("No active repositories to poll for {}", self.org_name);
            return Ok(());
        }

        info!(
            "Polling {} active repositories for {}",
            active_repos.len(),
            self.org_name
        );

        let semaphore = Arc::new(Semaphore::new(self.config.max_concurrent_repos));
        let mut poll_handles = Vec::new();

        for repo_state in active_repos {
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|e| anyhow::anyhow!("Failed to acquire semaphore permit: {e}"))?;
            let repo_clone = repo_state.clone();
            let repo_name = repo_state.repo_name.clone();
            let org_name = self.org_name.clone();
            let state_store = self.state_store.clone();
            let orchestrator = self.orchestrator.clone();
            let labels_filter = self.config.labels_filter.clone();

            let handle = tokio::spawn(async move {
                let result = Self::poll_repository(
                    &org_name,
                    repo_clone,
                    state_store,
                    orchestrator,
                    labels_filter,
                )
                .await;

                drop(permit);

                if let Err(e) = result {
                    error!("Error polling {}/{}: {}", org_name, repo_name, e);
                }
            });

            poll_handles.push(handle);
        }

        for handle in poll_handles {
            let _ = handle.await;
        }

        Ok(())
    }

    async fn poll_repository(
        org: &str,
        mut repo_state: RepoState,
        state_store: Arc<OrchestratorStateStore>,
        _orchestrator: Arc<Orchestrator>,
        _labels_filter: Option<String>,
    ) -> Result<()> {
        debug!("Polling repository: {}/{}", org, repo_state.repo_name);

        match Self::fetch_and_process_issues(org, &repo_state, &state_store).await {
            Ok(processed_count) => {
                repo_state.last_poll_at = Utc::now();
                repo_state.issues_processed += processed_count;
                repo_state.error_count = 0;
                repo_state.last_error = None;
                repo_state.status = RepoStatus::Active;

                state_store.upsert_repo_state(&repo_state).await?;

                if processed_count > 0 {
                    info!(
                        "Processed {} issues from {}/{}",
                        processed_count, org, repo_state.repo_name
                    );
                }
            }
            Err(e) => {
                error!("Failed to poll {}/{}: {}", org, repo_state.repo_name, e);

                repo_state.error_count += 1;
                repo_state.last_error = Some(e.to_string());
                repo_state.last_poll_at = Utc::now();

                if repo_state.error_count >= REPO_ERROR_THRESHOLD {
                    warn!(
                        "Repository {}/{} has {} errors, pausing",
                        org, repo_state.repo_name, repo_state.error_count
                    );
                    repo_state.status = RepoStatus::Paused;
                    repo_state.issues_failed += 1;
                }

                state_store.upsert_repo_state(&repo_state).await?;
            }
        }

        Ok(())
    }

    async fn fetch_and_process_issues(
        _org: &str,
        _repo_state: &RepoState,
        _state_store: &OrchestratorStateStore,
    ) -> Result<u64> {
        Ok(0)
    }

    fn clone_self(&self) -> Self {
        Self {
            org_name: self.org_name.clone(),
            orchestrator: self.orchestrator.clone(),
            discovery: self.discovery.clone(),
            state_store: self.state_store.clone(),
            config: OrgPollerConfig {
                poll_interval: self.config.poll_interval,
                max_concurrent_repos: self.config.max_concurrent_repos,
                discovery_interval: self.config.discovery_interval,
                repo_include_pattern: self.config.repo_include_pattern.clone(),
                repo_exclude_pattern: self.config.repo_exclude_pattern.clone(),
                include_archived: self.config.include_archived,
                labels_filter: self.config.labels_filter.clone(),
            },
            active_tasks: self.active_tasks.clone(),
            running: self.running.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;

    #[tokio::test]
    async fn test_org_poller_config_defaults() {
        let config = OrgPollerConfig::default();
        assert_eq!(config.poll_interval, Duration::from_secs(300));
        assert_eq!(config.max_concurrent_repos, 10);
        assert!(!config.include_archived);
    }
}
