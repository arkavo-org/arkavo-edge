use crate::error::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use std::path::Path;
use uuid::Uuid;

const DEFAULT_RETENTION_DAYS: i64 = 7;
const MAX_PROCESSED_ISSUES_PER_REPO: u64 = 5000;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum RepoStatus {
    Active,
    Paused,
    Failed,
}

impl std::fmt::Display for RepoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RepoStatus::Active => write!(f, "active"),
            RepoStatus::Paused => write!(f, "paused"),
            RepoStatus::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "TEXT", rename_all = "lowercase")]
pub enum IssueProcessingStatus {
    Pending,
    Processing,
    Completed,
    Failed,
}

impl std::fmt::Display for IssueProcessingStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IssueProcessingStatus::Pending => write!(f, "pending"),
            IssueProcessingStatus::Processing => write!(f, "processing"),
            IssueProcessingStatus::Completed => write!(f, "completed"),
            IssueProcessingStatus::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoState {
    pub org: String,
    pub repo_name: String,
    pub last_poll_at: DateTime<Utc>,
    pub issues_processed: u64,
    pub issues_failed: u64,
    pub last_error: Option<String>,
    pub error_count: u32,
    pub status: RepoStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedIssue {
    pub org: String,
    pub repo_name: String,
    pub issue_number: u64,
    pub task_id: Uuid,
    pub processed_at: DateTime<Utc>,
    pub status: IssueProcessingStatus,
    pub error_message: Option<String>,
    pub retry_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgStats {
    pub org_name: String,
    pub total_repos: usize,
    pub active_repos: usize,
    pub paused_repos: usize,
    pub failed_repos: usize,
    pub total_issues_processed: u64,
    pub total_issues_failed: u64,
    pub last_poll_at: Option<DateTime<Utc>>,
}

pub struct OrchestratorStateStore {
    pool: SqlitePool,
    retention_days: i64,
}

impl OrchestratorStateStore {
    pub async fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let db_path_str = db_path.display().to_string();
        let db_url = if db_path_str.starts_with("file:") {
            format!("sqlite://{db_path_str}")
        } else {
            format!("sqlite://{db_path_str}?mode=rwc")
        };

        let pool = SqlitePool::connect(&db_url).await?;

        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA busy_timeout = 5000")
            .execute(&pool)
            .await?;

        let store = Self {
            pool,
            retention_days: DEFAULT_RETENTION_DAYS,
        };

        store.init_schema().await?;

        Ok(store)
    }

    async fn init_schema(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS repo_state (
                org TEXT NOT NULL,
                repo_name TEXT NOT NULL,
                last_poll_at DATETIME NOT NULL,
                issues_processed INTEGER DEFAULT 0,
                issues_failed INTEGER DEFAULT 0,
                last_error TEXT,
                error_count INTEGER DEFAULT 0,
                status TEXT NOT NULL CHECK(status IN ('active', 'paused', 'failed')) DEFAULT 'active',
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (org, repo_name)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS processed_issues (
                org TEXT NOT NULL,
                repo_name TEXT NOT NULL,
                issue_number INTEGER NOT NULL,
                task_id TEXT NOT NULL,
                processed_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
                status TEXT NOT NULL CHECK(status IN ('pending', 'processing', 'completed', 'failed')),
                error_message TEXT,
                retry_count INTEGER DEFAULT 0,
                PRIMARY KEY (org, repo_name, issue_number)
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        // Migration: Add retry_count column if it doesn't exist (for existing databases)
        sqlx::query("ALTER TABLE processed_issues ADD COLUMN retry_count INTEGER DEFAULT 0")
            .execute(&self.pool)
            .await
            .ok(); // Ignore error if column already exists

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_processed_at ON processed_issues(processed_at)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_repo_status ON repo_state(status)")
            .execute(&self.pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_task_id ON processed_issues(task_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_org_repo_status ON processed_issues(org, repo_name, status)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            r#"
            CREATE TRIGGER IF NOT EXISTS update_repo_state_timestamp
            AFTER UPDATE ON repo_state
            BEGIN
                UPDATE repo_state SET updated_at = CURRENT_TIMESTAMP
                WHERE org = NEW.org AND repo_name = NEW.repo_name;
            END
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn upsert_repo_state(&self, state: &RepoState) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO repo_state (
                org, repo_name, last_poll_at, issues_processed, issues_failed,
                last_error, error_count, status
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(org, repo_name) DO UPDATE SET
                last_poll_at = excluded.last_poll_at,
                issues_processed = excluded.issues_processed,
                issues_failed = excluded.issues_failed,
                last_error = excluded.last_error,
                error_count = excluded.error_count,
                status = excluded.status
            "#,
        )
        .bind(&state.org)
        .bind(&state.repo_name)
        .bind(state.last_poll_at)
        .bind(state.issues_processed as i64)
        .bind(state.issues_failed as i64)
        .bind(&state.last_error)
        .bind(state.error_count as i64)
        .bind(state.status.to_string())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_repo_state(&self, org: &str, repo: &str) -> Result<Option<RepoState>> {
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                i64,
                i64,
                Option<String>,
                i64,
                String,
                String,
                String,
            ),
        >(
            r#"
            SELECT org, repo_name, last_poll_at, issues_processed, issues_failed,
                   last_error, error_count, status, created_at, updated_at
            FROM repo_state
            WHERE org = ? AND repo_name = ?
            "#,
        )
        .bind(org)
        .bind(repo)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            let parse_datetime = |s: &str| -> DateTime<Utc> {
                DateTime::parse_from_rfc3339(s)
                    .map(|dt| dt.with_timezone(&Utc))
                    .or_else(|_| {
                        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                            .map(|ndt| DateTime::from_naive_utc_and_offset(ndt, Utc))
                    })
                    .unwrap_or_else(|_| Utc::now())
            };

            RepoState {
                org: r.0,
                repo_name: r.1,
                last_poll_at: parse_datetime(&r.2),
                issues_processed: r.3 as u64,
                issues_failed: r.4 as u64,
                last_error: r.5,
                error_count: r.6 as u32,
                status: match r.7.as_str() {
                    "active" => RepoStatus::Active,
                    "paused" => RepoStatus::Paused,
                    "failed" => RepoStatus::Failed,
                    _ => RepoStatus::Active,
                },
                created_at: parse_datetime(&r.8),
                updated_at: parse_datetime(&r.9),
            }
        }))
    }

    pub async fn list_repos(
        &self,
        org: &str,
        status_filter: Option<RepoStatus>,
    ) -> Result<Vec<RepoState>> {
        let rows = if let Some(status) = status_filter {
            sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    i64,
                    i64,
                    Option<String>,
                    i64,
                    String,
                    String,
                    String,
                ),
            >(
                r#"
                SELECT org, repo_name, last_poll_at, issues_processed, issues_failed,
                       last_error, error_count, status, created_at, updated_at
                FROM repo_state
                WHERE org = ? AND status = ?
                ORDER BY repo_name
                "#,
            )
            .bind(org)
            .bind(status.to_string())
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<
                _,
                (
                    String,
                    String,
                    String,
                    i64,
                    i64,
                    Option<String>,
                    i64,
                    String,
                    String,
                    String,
                ),
            >(
                r#"
                SELECT org, repo_name, last_poll_at, issues_processed, issues_failed,
                       last_error, error_count, status, created_at, updated_at
                FROM repo_state
                WHERE org = ?
                ORDER BY repo_name
                "#,
            )
            .bind(org)
            .fetch_all(&self.pool)
            .await?
        };

        let parse_datetime = |s: &str| -> DateTime<Utc> {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                        .map(|ndt| DateTime::from_naive_utc_and_offset(ndt, Utc))
                })
                .unwrap_or_else(|_| Utc::now())
        };

        Ok(rows
            .into_iter()
            .map(|r| RepoState {
                org: r.0,
                repo_name: r.1,
                last_poll_at: parse_datetime(&r.2),
                issues_processed: r.3 as u64,
                issues_failed: r.4 as u64,
                last_error: r.5,
                error_count: r.6 as u32,
                status: match r.7.as_str() {
                    "active" => RepoStatus::Active,
                    "paused" => RepoStatus::Paused,
                    "failed" => RepoStatus::Failed,
                    _ => RepoStatus::Active,
                },
                created_at: parse_datetime(&r.8),
                updated_at: parse_datetime(&r.9),
            })
            .collect())
    }

    pub async fn mark_issue_processed(
        &self,
        org: &str,
        repo: &str,
        issue_number: u64,
        task_id: Uuid,
        status: IssueProcessingStatus,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO processed_issues (org, repo_name, issue_number, task_id, status)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(org, repo_name, issue_number) DO UPDATE SET
                task_id = excluded.task_id,
                status = excluded.status,
                processed_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(org)
        .bind(repo)
        .bind(issue_number as i64)
        .bind(task_id.to_string())
        .bind(status.to_string())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn is_issue_processed(
        &self,
        org: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<bool> {
        let result: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM processed_issues WHERE org = ? AND repo_name = ? AND issue_number = ?",
        )
        .bind(org)
        .bind(repo)
        .bind(issue_number as i64)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    pub async fn get_issue_task_id(
        &self,
        org: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<Option<Uuid>> {
        let result: Option<(String,)> = sqlx::query_as(
            "SELECT task_id FROM processed_issues WHERE org = ? AND repo_name = ? AND issue_number = ?",
        )
        .bind(org)
        .bind(repo)
        .bind(issue_number as i64)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.and_then(|(task_id,)| Uuid::parse_str(&task_id).ok()))
    }

    pub async fn cleanup_old_issues(&self, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query("DELETE FROM processed_issues WHERE processed_at < ?")
            .bind(before.to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    pub async fn cleanup_old_repos(&self, before: DateTime<Utc>) -> Result<u64> {
        let result = sqlx::query("DELETE FROM repo_state WHERE updated_at < ?")
            .bind(before.to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// Get all issues with a given processing status
    pub async fn get_issues_by_status(
        &self,
        status: IssueProcessingStatus,
    ) -> Result<Vec<ProcessedIssue>> {
        let rows = sqlx::query_as::<
            _,
            (String, String, i64, String, String, String, Option<String>, i64),
        >(
            r#"
            SELECT org, repo_name, issue_number, task_id, processed_at, status, error_message, retry_count
            FROM processed_issues
            WHERE status = ?
            ORDER BY processed_at DESC
            "#,
        )
        .bind(status.to_string())
        .fetch_all(&self.pool)
        .await?;

        let parse_datetime = |s: &str| -> DateTime<Utc> {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                        .map(|ndt| DateTime::from_naive_utc_and_offset(ndt, Utc))
                })
                .unwrap_or_else(|_| Utc::now())
        };

        Ok(rows
            .into_iter()
            .map(|r| ProcessedIssue {
                org: r.0,
                repo_name: r.1,
                issue_number: r.2 as u64,
                task_id: Uuid::parse_str(&r.3).unwrap_or_else(|_| Uuid::nil()),
                processed_at: parse_datetime(&r.4),
                status: match r.5.as_str() {
                    "pending" => IssueProcessingStatus::Pending,
                    "processing" => IssueProcessingStatus::Processing,
                    "completed" => IssueProcessingStatus::Completed,
                    "failed" => IssueProcessingStatus::Failed,
                    _ => IssueProcessingStatus::Pending,
                },
                error_message: r.6,
                retry_count: r.7 as u32,
            })
            .collect())
    }

    /// Update the status of a processed issue
    pub async fn update_issue_status(
        &self,
        org: &str,
        repo: &str,
        issue_number: u64,
        status: IssueProcessingStatus,
        error_message: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE processed_issues
            SET status = ?, error_message = ?, processed_at = CURRENT_TIMESTAMP
            WHERE org = ? AND repo_name = ? AND issue_number = ?
            "#,
        )
        .bind(status.to_string())
        .bind(error_message)
        .bind(org)
        .bind(repo)
        .bind(issue_number as i64)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Increment retry count and return the new count
    pub async fn increment_retry_count(
        &self,
        org: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<u32> {
        sqlx::query(
            r#"
            UPDATE processed_issues
            SET retry_count = retry_count + 1
            WHERE org = ? AND repo_name = ? AND issue_number = ?
            "#,
        )
        .bind(org)
        .bind(repo)
        .bind(issue_number as i64)
        .execute(&self.pool)
        .await?;

        // Get the new retry count
        let result: Option<(i64,)> = sqlx::query_as(
            "SELECT retry_count FROM processed_issues WHERE org = ? AND repo_name = ? AND issue_number = ?",
        )
        .bind(org)
        .bind(repo)
        .bind(issue_number as i64)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(|(count,)| count as u32).unwrap_or(0))
    }

    pub async fn get_org_stats(&self, org: &str) -> Result<OrgStats> {
        let repo_stats: Vec<(i64, String)> =
            sqlx::query_as("SELECT COUNT(*), status FROM repo_state WHERE org = ? GROUP BY status")
                .bind(org)
                .fetch_all(&self.pool)
                .await?;

        let mut stats = OrgStats {
            org_name: org.to_string(),
            total_repos: 0,
            active_repos: 0,
            paused_repos: 0,
            failed_repos: 0,
            total_issues_processed: 0,
            total_issues_failed: 0,
            last_poll_at: None,
        };

        for (count, status) in repo_stats {
            stats.total_repos += count as usize;
            match status.as_str() {
                "active" => stats.active_repos = count as usize,
                "paused" => stats.paused_repos = count as usize,
                "failed" => stats.failed_repos = count as usize,
                _ => {}
            }
        }

        let issue_stats: Option<(i64, i64)> = sqlx::query_as(
            r#"
            SELECT
                SUM(issues_processed),
                SUM(issues_failed)
            FROM repo_state
            WHERE org = ?
            "#,
        )
        .bind(org)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((processed, failed)) = issue_stats {
            stats.total_issues_processed = processed as u64;
            stats.total_issues_failed = failed as u64;
        }

        let last_poll: Option<(String,)> = sqlx::query_as(
            "SELECT last_poll_at FROM repo_state WHERE org = ? ORDER BY last_poll_at DESC LIMIT 1",
        )
        .bind(org)
        .fetch_optional(&self.pool)
        .await?;

        if let Some((poll_time,)) = last_poll {
            stats.last_poll_at = DateTime::parse_from_rfc3339(&poll_time)
                .ok()
                .map(|dt| dt.with_timezone(&Utc));
        }

        Ok(stats)
    }

    pub async fn prune_large_repos(&self) -> Result<()> {
        let large_repos: Vec<(String, String, i64)> = sqlx::query_as(
            r#"
            SELECT pi.org, pi.repo_name, COUNT(*) as issue_count
            FROM processed_issues pi
            GROUP BY pi.org, pi.repo_name
            HAVING issue_count > ?
            "#,
        )
        .bind(MAX_PROCESSED_ISSUES_PER_REPO as i64)
        .fetch_all(&self.pool)
        .await?;

        for (org, repo, count) in large_repos {
            let to_delete = count - MAX_PROCESSED_ISSUES_PER_REPO as i64;

            sqlx::query(
                r#"
                DELETE FROM processed_issues
                WHERE org = ? AND repo_name = ? AND issue_number IN (
                    SELECT issue_number FROM processed_issues
                    WHERE org = ? AND repo_name = ?
                    ORDER BY processed_at ASC
                    LIMIT ?
                )
                "#,
            )
            .bind(&org)
            .bind(&repo)
            .bind(&org)
            .bind(&repo)
            .bind(to_delete)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    pub async fn run_maintenance(&self) -> Result<()> {
        let cutoff = Utc::now() - Duration::days(self.retention_days);

        self.cleanup_old_issues(cutoff).await?;
        self.prune_large_repos().await?;

        sqlx::query("VACUUM").execute(&self.pool).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    async fn create_test_store() -> OrchestratorStateStore {
        static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
        let test_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        // Use sqlite memory database for tests
        let db_path = std::path::PathBuf::from(format!(
            "file:arkavo_orchestrator_test_{timestamp}_{test_id}?mode=memory&cache=shared"
        ));

        OrchestratorStateStore::new(&db_path).await.unwrap()
    }

    #[tokio::test]
    async fn test_upsert_repo_state() {
        let store = create_test_store().await;

        let state = RepoState {
            org: "test-org".to_string(),
            repo_name: "test-repo".to_string(),
            last_poll_at: Utc::now(),
            issues_processed: 10,
            issues_failed: 2,
            last_error: None,
            error_count: 0,
            status: RepoStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        store.upsert_repo_state(&state).await.unwrap();

        let retrieved = store
            .get_repo_state("test-org", "test-repo")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(retrieved.org, "test-org");
        assert_eq!(retrieved.repo_name, "test-repo");
        assert_eq!(retrieved.issues_processed, 10);
    }

    #[tokio::test]
    async fn test_mark_issue_processed() {
        let store = create_test_store().await;

        let task_id = Uuid::new_v4();
        store
            .mark_issue_processed(
                "test-org",
                "test-repo",
                42,
                task_id,
                IssueProcessingStatus::Completed,
            )
            .await
            .unwrap();

        let is_processed = store
            .is_issue_processed("test-org", "test-repo", 42)
            .await
            .unwrap();

        assert!(is_processed);

        let retrieved_task_id = store
            .get_issue_task_id("test-org", "test-repo", 42)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(retrieved_task_id, task_id);
    }

    #[tokio::test]
    async fn test_org_stats() {
        let store = create_test_store().await;

        for i in 0..5 {
            let state = RepoState {
                org: "test-org".to_string(),
                repo_name: format!("repo-{}", i),
                last_poll_at: Utc::now(),
                issues_processed: i * 10,
                issues_failed: i,
                last_error: None,
                error_count: 0,
                status: if i < 3 {
                    RepoStatus::Active
                } else {
                    RepoStatus::Paused
                },
                created_at: Utc::now(),
                updated_at: Utc::now(),
            };
            store.upsert_repo_state(&state).await.unwrap();
        }

        let stats = store.get_org_stats("test-org").await.unwrap();

        assert_eq!(stats.total_repos, 5);
        assert_eq!(stats.active_repos, 3);
        assert_eq!(stats.paused_repos, 2);
    }
}
