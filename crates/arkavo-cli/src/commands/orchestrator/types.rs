use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use super::constants::MAX_PROCESSED_ISSUES;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct PollState {
    pub last_poll: DateTime<Utc>,
    processed_issues: HashMap<u64, DateTime<Utc>>,
    #[serde(default = "default_max_processed")]
    pub max_processed_issues: usize,
}

fn default_max_processed() -> usize {
    MAX_PROCESSED_ISSUES
}

impl PollState {
    pub(super) fn new() -> Self {
        Self {
            last_poll: Utc::now(),
            processed_issues: HashMap::new(),
            max_processed_issues: MAX_PROCESSED_ISSUES,
        }
    }

    pub(super) fn contains(&self, issue_number: u64) -> bool {
        self.processed_issues.contains_key(&issue_number)
    }

    pub(super) fn insert_processed(&mut self, issue_number: u64) {
        self.processed_issues.insert(issue_number, Utc::now());

        if self.processed_issues.len() > self.max_processed_issues {
            let mut entries: Vec<_> = self
                .processed_issues
                .iter()
                .map(|(num, time)| (*num, *time))
                .collect();
            entries.sort_by_key(|(_, timestamp)| *timestamp);

            let to_remove_count = self.processed_issues.len() - self.max_processed_issues;
            let to_remove: Vec<u64> = entries
                .iter()
                .take(to_remove_count)
                .map(|(num, _)| *num)
                .collect();

            for number in to_remove {
                self.processed_issues.remove(&number);
            }
        }
    }

    fn state_file_path(repo: &str) -> PathBuf {
        let repo_safe = repo.replace('/', "_");
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".arkavo")
            .join(format!("orchestrator-poll-{repo_safe}.json"))
    }

    pub(super) fn load(repo: &str) -> Result<Self> {
        let path = Self::state_file_path(repo);
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(Self::new())
        }
    }

    pub(super) fn save(&self, repo: &str) -> Result<()> {
        let path = Self::state_file_path(repo);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }
}

impl Default for PollState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct GitHubIssue {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: String,
    pub labels: Vec<GitHubLabel>,
    pub user: GitHubUser,
    pub assignees: Vec<GitHubUser>,
    pub id: u64,
    pub closed_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GitHubLabel {
    pub id: u64,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GitHubUser {
    pub id: u64,
    pub login: String,
    #[serde(rename = "type")]
    pub user_type: String,
    pub avatar_url: String,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct GitHubRepository {
    pub id: u64,
    pub name: String,
    pub full_name: String,
    pub owner: GitHubUser,
    pub html_url: String,
    pub description: Option<String>,
    pub default_branch: String,
    pub language: Option<String>,
}
