//! SQLite persistence for ensemble state
//!
//! Stores candidates and regret statistics for durability.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::candidate::{CandidateState, GenerationMethod, PolicyCandidate};
use crate::error::{EnsembleError, EnsembleResult};
use crate::regret::CandidateRegret;

/// Stored candidate row
#[derive(Debug, FromRow)]
struct CandidateRow {
    id: String,
    policy_graph: String,
    state: String,
    generation_method: String,
    created_at: String,
    first_flagged_at: Option<String>,
    evaluation_count: i64,
    invariant_violations: i32,
}

/// Stored regret row
#[derive(Debug, FromRow)]
struct RegretRow {
    candidate_id: String,
    cumulative_regret: f64,
    outcome_count: i64,
    mean_regret: f64,
    variance: f64,
}

/// SQLite storage for ensemble data
pub struct SqliteEnsembleStore {
    pool: SqlitePool,
}

impl SqliteEnsembleStore {
    /// Create a new store with the given connection pool
    pub async fn new(pool: SqlitePool) -> EnsembleResult<Self> {
        let store = Self { pool };
        store.init_schema().await?;
        Ok(store)
    }

    /// Initialize the database schema
    async fn init_schema(&self) -> EnsembleResult<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS ensemble_candidates (
                id TEXT PRIMARY KEY,
                policy_graph TEXT NOT NULL,
                state TEXT NOT NULL,
                generation_method TEXT NOT NULL,
                created_at TEXT NOT NULL,
                first_flagged_at TEXT,
                evaluation_count INTEGER NOT NULL DEFAULT 0,
                invariant_violations INTEGER NOT NULL DEFAULT 0
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS ensemble_regret (
                candidate_id TEXT PRIMARY KEY,
                cumulative_regret REAL NOT NULL,
                outcome_count INTEGER NOT NULL,
                mean_regret REAL NOT NULL,
                variance REAL NOT NULL,
                FOREIGN KEY (candidate_id) REFERENCES ensemble_candidates(id)
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Store a candidate
    pub async fn store_candidate(&self, candidate: &PolicyCandidate) -> EnsembleResult<()> {
        let policy_graph = serde_json::to_string(&SerializableGraph::from(&candidate.policy.graph))
            .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        let state = format!("{:?}", candidate.state);

        let generation_method = serde_json::to_string(&candidate.generation_method)
            .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        let first_flagged = candidate.first_flagged_at.map(|t| t.to_rfc3339());

        sqlx::query(
            r#"
            INSERT OR REPLACE INTO ensemble_candidates
            (id, policy_graph, state, generation_method, created_at, first_flagged_at,
             evaluation_count, invariant_violations)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(candidate.id.to_string())
        .bind(policy_graph)
        .bind(state)
        .bind(generation_method)
        .bind(candidate.created_at.to_rfc3339())
        .bind(first_flagged)
        .bind(candidate.evaluation_count as i64)
        .bind(candidate.invariant_violations as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Store regret statistics
    pub async fn store_regret(&self, regret: &CandidateRegret) -> EnsembleResult<()> {
        sqlx::query(
            r#"
            INSERT OR REPLACE INTO ensemble_regret
            (candidate_id, cumulative_regret, outcome_count, mean_regret, variance)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
        )
        .bind(regret.candidate_id.to_string())
        .bind(regret.cumulative_regret)
        .bind(regret.outcome_count as i64)
        .bind(regret.mean_regret)
        .bind(regret.variance())
        .execute(&self.pool)
        .await
        .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        Ok(())
    }

    /// Load all candidates
    pub async fn load_candidates(&self) -> EnsembleResult<Vec<StoredCandidate>> {
        let rows: Vec<CandidateRow> = sqlx::query_as(
            "SELECT * FROM ensemble_candidates WHERE state NOT IN ('Promoted', 'Discarded')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        let mut candidates = Vec::with_capacity(rows.len());
        for row in rows {
            candidates.push(StoredCandidate::from_row(row)?);
        }

        Ok(candidates)
    }

    /// Load regret for a candidate
    pub async fn load_regret(&self, candidate_id: Uuid) -> EnsembleResult<Option<StoredRegret>> {
        let row: Option<RegretRow> = sqlx::query_as(
            "SELECT * FROM ensemble_regret WHERE candidate_id = ?1",
        )
        .bind(candidate_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        Ok(row.map(StoredRegret::from_row))
    }

    /// Delete a candidate and its regret
    pub async fn delete_candidate(&self, candidate_id: Uuid) -> EnsembleResult<()> {
        sqlx::query("DELETE FROM ensemble_regret WHERE candidate_id = ?1")
            .bind(candidate_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        sqlx::query("DELETE FROM ensemble_candidates WHERE id = ?1")
            .bind(candidate_id.to_string())
            .execute(&self.pool)
            .await
            .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        Ok(())
    }
}

/// Stored candidate data (without policy graph deserialized)
#[derive(Debug)]
pub struct StoredCandidate {
    pub id: Uuid,
    pub policy_graph_json: String,
    pub state: CandidateState,
    pub generation_method: GenerationMethod,
    pub created_at: DateTime<Utc>,
    pub first_flagged_at: Option<DateTime<Utc>>,
    pub evaluation_count: u64,
    pub invariant_violations: u32,
}

impl StoredCandidate {
    fn from_row(row: CandidateRow) -> EnsembleResult<Self> {
        let id = Uuid::parse_str(&row.id)
            .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        let state = parse_state(&row.state)?;

        let generation_method: GenerationMethod = serde_json::from_str(&row.generation_method)
            .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        let created_at = DateTime::parse_from_rfc3339(&row.created_at)
            .map(|t| t.with_timezone(&Utc))
            .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        let first_flagged_at = row
            .first_flagged_at
            .map(|s| DateTime::parse_from_rfc3339(&s).map(|t| t.with_timezone(&Utc)))
            .transpose()
            .map_err(|e| EnsembleError::Storage(e.to_string()))?;

        Ok(Self {
            id,
            policy_graph_json: row.policy_graph,
            state,
            generation_method,
            created_at,
            first_flagged_at,
            evaluation_count: row.evaluation_count as u64,
            invariant_violations: row.invariant_violations as u32,
        })
    }
}

/// Stored regret data
#[derive(Debug)]
pub struct StoredRegret {
    pub candidate_id: Uuid,
    pub cumulative_regret: f64,
    pub outcome_count: u64,
    pub mean_regret: f64,
    pub variance: f64,
}

impl StoredRegret {
    fn from_row(row: RegretRow) -> Self {
        Self {
            candidate_id: Uuid::parse_str(&row.candidate_id).unwrap_or_default(),
            cumulative_regret: row.cumulative_regret,
            outcome_count: row.outcome_count as u64,
            mean_regret: row.mean_regret,
            variance: row.variance,
        }
    }
}

fn parse_state(s: &str) -> EnsembleResult<CandidateState> {
    match s {
        "Pending" => Ok(CandidateState::Pending),
        "Active" => Ok(CandidateState::Active),
        "FlaggedForPromotion" => Ok(CandidateState::FlaggedForPromotion),
        "Promoted" => Ok(CandidateState::Promoted),
        "Discarded" => Ok(CandidateState::Discarded),
        _ => Err(EnsembleError::Storage(format!("Unknown state: {}", s))),
    }
}

/// Serializable graph wrapper for JSON storage
#[derive(serde::Serialize, serde::Deserialize)]
struct SerializableGraph {
    inputs: Vec<u16>,
    outputs: Vec<u16>,
    nodes_count: usize,
}

impl From<&torg_core::Graph> for SerializableGraph {
    fn from(graph: &torg_core::Graph) -> Self {
        Self {
            inputs: graph.inputs.clone(),
            outputs: graph.outputs.clone(),
            nodes_count: graph.nodes.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_state() {
        assert_eq!(parse_state("Pending").unwrap(), CandidateState::Pending);
        assert_eq!(parse_state("Active").unwrap(), CandidateState::Active);
        assert_eq!(
            parse_state("FlaggedForPromotion").unwrap(),
            CandidateState::FlaggedForPromotion
        );
        assert!(parse_state("Unknown").is_err());
    }
}
