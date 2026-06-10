//! Read existing episode traces from the learning store, strip observe calls
//! per the lesson-synthesis rule, and group them into per-category batches.
//!
//! The store is opened **read-only** and never created — the job touches the
//! on-disk traces only as data, never as a live runtime.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use serde::Deserialize;
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

/// The subset of the on-disk `Observation` JSON the consolidation job needs.
/// Matches the runtime `Observation` serialization (extra fields are ignored).
#[derive(Debug, Deserialize)]
struct RawObservation {
    #[serde(default)]
    tools_used: Vec<String>,
}

/// The subset of the on-disk `EpisodeOutcome` JSON the job needs.
#[derive(Debug, Deserialize)]
struct RawOutcome {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    quality_metrics: RawQuality,
}

#[derive(Debug, Default, Deserialize)]
struct RawQuality {
    #[serde(default)]
    correctness: f64,
}

/// One action→outcome pair, with read-only/observe tools already stripped.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionOutcome {
    /// Action tools used (observe/list/hash/summary removed).
    pub actions: Vec<String>,
    pub success: bool,
    /// Correctness quality metric in `[0, 1]`.
    pub quality: f64,
}

/// All action→outcome evidence for a single task category.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryBatch {
    pub category: String,
    /// Store row ids of the episodes in this batch, preserved so tonight's
    /// proposals can be back-linked to their evidence later.
    pub episode_ids: Vec<String>,
    pub outcomes: Vec<ActionOutcome>,
}

/// Outcome of loading + grouping episodes.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadResult {
    /// Categories that met the minimum-episode threshold, ready to consolidate.
    pub batches: Vec<CategoryBatch>,
    pub episodes_total: u64,
    pub episodes_consolidated: u64,
    pub categories_total: u64,
    pub categories_skipped: u64,
}

/// The lesson-synthesis rule: keep only *action* tools.
///
/// A tool is read-only (and dropped) when its name contains observe/list/hash/
/// summary. Lessons must reason about which actions produce good outcomes, not
/// about tool-calling mechanics.
#[must_use]
pub fn is_action_tool(tool: &str) -> bool {
    let lower = tool.to_lowercase();
    !lower.contains("observe")
        && !lower.contains("list")
        && !lower.contains("hash")
        && !lower.contains("summary")
}

/// Apply [`is_action_tool`] to a tool list.
fn strip_observe(tools: &[String]) -> Vec<String> {
    tools
        .iter()
        .filter(|t| is_action_tool(t))
        .cloned()
        .collect()
}

/// Group raw `(id, category, observation_json, outcome_json)` rows into
/// batches, stripping observe calls and dropping categories below
/// `min_episodes`.
fn group_rows(rows: Vec<(String, String, String, String)>, min_episodes: usize) -> LoadResult {
    let episodes_total = rows.len() as u64;
    // BTreeMap keeps category order stable for reproducible output.
    let mut by_category: BTreeMap<String, (Vec<String>, Vec<ActionOutcome>)> = BTreeMap::new();

    for (id, category, obs_json, outcome_json) in rows {
        let actions = serde_json::from_str::<RawObservation>(&obs_json)
            .map(|o| strip_observe(&o.tools_used))
            .unwrap_or_default();
        let (success, quality) = serde_json::from_str::<RawOutcome>(&outcome_json)
            .map(|o| (o.success, o.quality_metrics.correctness.clamp(0.0, 1.0)))
            .unwrap_or((false, 0.0));
        let (ids, outcomes) = by_category.entry(category).or_default();
        ids.push(id);
        outcomes.push(ActionOutcome {
            actions,
            success,
            quality,
        });
    }

    let categories_total = by_category.len() as u64;
    let mut batches = Vec::new();
    let mut categories_skipped = 0_u64;
    let mut episodes_consolidated = 0_u64;

    for (category, (episode_ids, outcomes)) in by_category {
        if outcomes.len() >= min_episodes {
            episodes_consolidated += outcomes.len() as u64;
            batches.push(CategoryBatch {
                category,
                episode_ids,
                outcomes,
            });
        } else {
            categories_skipped += 1;
        }
    }

    LoadResult {
        batches,
        episodes_total,
        episodes_consolidated,
        categories_total,
        categories_skipped,
    }
}

/// Read-only reader over the episode SQLite store.
#[derive(Debug)]
pub struct EpisodeReader {
    pool: sqlx::SqlitePool,
}

impl EpisodeReader {
    /// Open the learning store read-only. Errors if the file does not exist —
    /// the job never creates or migrates the store.
    pub async fn open(db_path: &Path) -> anyhow::Result<Self> {
        anyhow::ensure!(
            db_path.exists(),
            "episode store does not exist: {}",
            db_path.display()
        );
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .read_only(true)
            .create_if_missing(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .with_context(|| format!("opening episode store {}", db_path.display()))?;
        Ok(Self { pool })
    }

    /// Load and group all episodes into per-category batches.
    pub async fn load_batches(&self, min_episodes: usize) -> anyhow::Result<LoadResult> {
        let rows =
            sqlx::query("SELECT id, task_category, observation_json, outcome_json FROM episodes")
                .fetch_all(&self.pool)
                .await
                .context("querying episodes table")?;

        let raw: Vec<(String, String, String, String)> = rows
            .into_iter()
            .map(|row| {
                Ok::<_, sqlx::Error>((
                    row.try_get::<String, _>("id")?,
                    row.try_get::<String, _>("task_category")?,
                    row.try_get::<String, _>("observation_json")?,
                    row.try_get::<String, _>("outcome_json")?,
                ))
            })
            .collect::<Result<_, _>>()
            .context("decoding episode rows")?;

        Ok(group_rows(raw, min_episodes))
    }
}

#[cfg(test)]
mod tests {
    // #[tokio::test] expands to Runtime::block_on; harmless in tests.
    #![allow(clippy::disallowed_methods)]

    use super::*;

    #[test]
    fn strips_read_only_tools() {
        let tools = vec![
            "observe".to_string(),
            "list_tools".to_string(),
            "stateHash".to_string(),
            "episodeSummary".to_string(),
            "game-rl:step".to_string(),
            "set_priority".to_string(),
        ];
        assert_eq!(
            strip_observe(&tools),
            vec!["game-rl:step".to_string(), "set_priority".to_string()]
        );
    }

    #[test]
    fn action_tool_rule_matches_runtime() {
        assert!(!is_action_tool("Observe"));
        assert!(!is_action_tool("list_resources"));
        assert!(!is_action_tool("stateHash"));
        assert!(!is_action_tool("episodeSummary"));
        assert!(is_action_tool("game-rl:reset"));
        assert!(is_action_tool("set_priority"));
    }

    #[test]
    fn grouping_drops_below_threshold() {
        let rows = vec![
            (
                "ep-1".to_string(),
                "a".to_string(),
                r#"{"tools_used":["observe","step"]}"#.to_string(),
                r#"{"success":true,"quality_metrics":{"correctness":0.9}}"#.to_string(),
            ),
            (
                "ep-2".to_string(),
                "a".to_string(),
                r#"{"tools_used":["step"]}"#.to_string(),
                r#"{"success":false,"quality_metrics":{"correctness":0.2}}"#.to_string(),
            ),
            (
                "ep-3".to_string(),
                "a".to_string(),
                r#"{"tools_used":["reset"]}"#.to_string(),
                r#"{"success":true,"quality_metrics":{"correctness":0.5}}"#.to_string(),
            ),
            (
                "ep-4".to_string(),
                "b".to_string(),
                r#"{"tools_used":["step"]}"#.to_string(),
                r#"{"success":true,"quality_metrics":{"correctness":1.0}}"#.to_string(),
            ),
        ];
        let result = group_rows(rows, 3);
        assert_eq!(result.episodes_total, 4);
        assert_eq!(result.categories_total, 2);
        assert_eq!(result.batches.len(), 1);
        assert_eq!(result.batches[0].category, "a");
        assert_eq!(result.episodes_consolidated, 3);
        assert_eq!(result.categories_skipped, 1);
        // Episode ids preserved so proposals can be back-linked to evidence.
        assert_eq!(
            result.batches[0].episode_ids,
            vec!["ep-1".to_string(), "ep-2".to_string(), "ep-3".to_string()]
        );
        // observe stripped from the first episode.
        assert_eq!(
            result.batches[0].outcomes[0].actions,
            vec!["step".to_string()]
        );
    }

    #[test]
    fn malformed_json_yields_empty_action_failed_outcome() {
        let rows = vec![
            (
                "ep-1".to_string(),
                "a".to_string(),
                "not json".to_string(),
                "not json".to_string(),
            ),
            (
                "ep-2".to_string(),
                "a".to_string(),
                "{}".to_string(),
                "{}".to_string(),
            ),
            (
                "ep-3".to_string(),
                "a".to_string(),
                r#"{"tools_used":["step"]}"#.to_string(),
                r#"{"success":true,"quality_metrics":{"correctness":0.7}}"#.to_string(),
            ),
        ];
        let result = group_rows(rows, 1);
        assert_eq!(result.batches.len(), 1);
        let outcomes = &result.batches[0].outcomes;
        assert!(outcomes[0].actions.is_empty());
        assert!(!outcomes[0].success);
        assert!((outcomes[0].quality).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn reads_episodes_from_sqlite() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        // Seed a store with the runtime episodes schema.
        let opts = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE episodes (id TEXT PRIMARY KEY, agent_id TEXT, swarm_id TEXT, \
             task_id TEXT, task_category TEXT NOT NULL, observation_json TEXT NOT NULL, \
             outcome_json TEXT NOT NULL, timestamp TEXT, context_hash BLOB)",
        )
        .execute(&pool)
        .await
        .unwrap();
        for i in 0..3 {
            sqlx::query(
                "INSERT INTO episodes (id, task_category, observation_json, outcome_json) \
                 VALUES (?, ?, ?, ?)",
            )
            .bind(format!("id-{i}"))
            .bind("colony")
            .bind(r#"{"tools_used":["observe","set_priority"]}"#)
            .bind(r#"{"success":true,"quality_metrics":{"correctness":0.8}}"#)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool.close().await;

        let reader = EpisodeReader::open(&path).await.unwrap();
        let result = reader.load_batches(3).await.unwrap();
        assert_eq!(result.episodes_total, 3);
        assert_eq!(result.batches.len(), 1);
        assert_eq!(result.batches[0].category, "colony");
        assert_eq!(result.batches[0].episode_ids.len(), 3);
        assert!(result.batches[0].episode_ids.contains(&"id-0".to_string()));
        assert_eq!(
            result.batches[0].outcomes[0].actions,
            vec!["set_priority".to_string()]
        );
    }

    #[tokio::test]
    async fn open_missing_store_errors() {
        let err = EpisodeReader::open(Path::new("/nonexistent/does-not-exist.db"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }
}
