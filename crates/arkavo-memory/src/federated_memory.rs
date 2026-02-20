use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqlitePool;
use std::path::Path;

/// ABAC attribute requirement for memory access control.
///
/// Mirrors the TDF policy `Attribute` type without requiring the TDF crate
/// as a dependency. Entitlement matching follows the same FQN convention:
/// `{authority}/attr/{name}/value/{value}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryAttribute {
    pub attribute: String,
    pub values: Vec<String>,
}

/// Access policy attached to federated memory items.
///
/// All attributes must be satisfied (AND semantics).
/// Each attribute allows any of its listed values (OR within attribute).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryPolicy {
    pub attributes: Vec<MemoryAttribute>,
}

/// A stored federated memory item with access policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedItem {
    pub id: String,
    pub agent_id: String,
    pub session_id: String,
    pub content_type: String,
    pub content: Vec<u8>,
    pub summary: String,
    pub token_count: usize,
    pub policy: MemoryPolicy,
    pub created_at: DateTime<Utc>,
}

/// Query parameters for federated memory retrieval.
#[derive(Debug, Clone)]
pub struct FederatedQuery {
    pub requester_id: String,
    pub entitlements: Vec<String>,
    pub session_filter: Option<String>,
    pub agent_filter: Option<String>,
    pub content_type_filter: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub limit: usize,
}

/// Result of a federated memory query.
#[derive(Debug)]
pub struct FederatedResult {
    pub items: Vec<FederatedItem>,
    pub total_matched: usize,
    pub filtered_by_policy: usize,
}

/// Summary of available federated memory for gossip announcements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManifest {
    pub agent_id: String,
    pub item_count: usize,
    pub total_tokens: usize,
    pub content_types: Vec<String>,
    pub oldest: Option<DateTime<Utc>>,
    pub newest: Option<DateTime<Utc>>,
}

/// Evaluate whether entitlements satisfy a memory policy.
///
/// Returns `true` if all policy attributes are satisfied.
/// Follows the same matching rules as `arkavo_tdf::AbacEvaluator`:
/// - Each attribute must have at least one matching entitlement
/// - Entitlement FQN: `{authority}/attr/{name}/value/{value}`
/// - Policy attribute FQN: `{authority}/attr/{name}` with values list
pub fn evaluate_entitlements(entitlements: &[String], policy: &MemoryPolicy) -> bool {
    if policy.attributes.is_empty() {
        return true;
    }

    policy.attributes.iter().all(|attr| {
        attr.values.iter().any(|required_value| {
            let expected = format!("{}/value/{required_value}", attr.attribute);
            entitlements.contains(&expected)
        })
    })
}

/// SQLite-backed service for ABAC-scoped federated memory retrieval.
///
/// Stores memory items with attached ABAC policies and filters queries
/// based on pre-verified entitlements from the transport layer.
pub struct FederatedMemoryService {
    pool: SqlitePool,
}

impl FederatedMemoryService {
    /// Create a new service at the given database path.
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

        let svc = Self { pool };
        svc.init_schema().await?;
        Ok(svc)
    }

    async fn init_schema(&self) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS federated_memory (
                id TEXT PRIMARY KEY,
                agent_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                content_type TEXT NOT NULL,
                content BLOB NOT NULL,
                summary TEXT NOT NULL,
                token_count INTEGER NOT NULL,
                policy_json TEXT NOT NULL,
                created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(&self.pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_fed_mem_agent ON federated_memory (agent_id)")
            .execute(&self.pool)
            .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_fed_mem_session ON federated_memory (session_id)",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_fed_mem_created ON federated_memory (created_at)",
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Store a batch of memory items with their ABAC policies.
    pub async fn store_batch(&self, items: &[FederatedItem]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for item in items {
            let policy_json = serde_json::to_string(&item.policy)
                .unwrap_or_else(|_| r#"{"attributes":[]}"#.to_string());

            sqlx::query(
                r#"
                INSERT OR REPLACE INTO federated_memory
                    (id, agent_id, session_id, content_type, content,
                     summary, token_count, policy_json, created_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&item.id)
            .bind(&item.agent_id)
            .bind(&item.session_id)
            .bind(&item.content_type)
            .bind(&item.content)
            .bind(&item.summary)
            .bind(item.token_count as i64)
            .bind(&policy_json)
            .bind(item.created_at.to_rfc3339())
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Query memory items filtered by ABAC entitlements.
    ///
    /// Items that the requester lacks entitlements for are excluded from
    /// results but counted in `filtered_by_policy`.
    pub async fn query(&self, q: &FederatedQuery) -> Result<FederatedResult> {
        let rows = self.fetch_candidates(q).await?;

        let total_matched = rows.len();
        let mut items = Vec::new();
        let mut filtered = 0;

        for row in rows {
            let policy: MemoryPolicy = serde_json::from_str(&row.policy_json).unwrap_or_default();

            if evaluate_entitlements(&q.entitlements, &policy) {
                items.push(FederatedItem {
                    id: row.id,
                    agent_id: row.agent_id,
                    session_id: row.session_id,
                    content_type: row.content_type,
                    content: row.content,
                    summary: row.summary,
                    token_count: row.token_count as usize,
                    policy,
                    created_at: DateTime::parse_from_rfc3339(&row.created_at)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                });
            } else {
                filtered += 1;
            }
        }

        Ok(FederatedResult {
            items,
            total_matched,
            filtered_by_policy: filtered,
        })
    }

    async fn fetch_candidates(&self, q: &FederatedQuery) -> Result<Vec<RawRow>> {
        // Build dynamic query based on filters
        let mut sql = String::from(
            "SELECT id, agent_id, session_id, content_type, content, \
                    summary, token_count, policy_json, created_at \
             FROM federated_memory WHERE 1=1",
        );
        let mut binds: Vec<String> = Vec::new();

        if let Some(ref session) = q.session_filter {
            sql.push_str(" AND session_id = ?");
            binds.push(session.clone());
        }
        if let Some(ref agent) = q.agent_filter {
            sql.push_str(" AND agent_id = ?");
            binds.push(agent.clone());
        }
        if let Some(ref ct) = q.content_type_filter {
            sql.push_str(" AND content_type = ?");
            binds.push(ct.clone());
        }
        if let Some(since) = q.since {
            sql.push_str(" AND created_at > ?");
            binds.push(since.to_rfc3339());
        }

        sql.push_str(" ORDER BY created_at DESC LIMIT ?");
        binds.push(q.limit.to_string());

        let mut query = sqlx::query_as::<_, RawRow>(&sql);
        for bind in &binds {
            query = query.bind(bind);
        }

        Ok(query.fetch_all(&self.pool).await?)
    }

    /// Build a context manifest summarizing available memory.
    pub async fn build_manifest(&self, agent_id: &str) -> Result<ContextManifest> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM federated_memory WHERE agent_id = ?")
                .bind(agent_id)
                .fetch_one(&self.pool)
                .await?;

        let (total_tokens,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(token_count), 0) FROM federated_memory WHERE agent_id = ?",
        )
        .bind(agent_id)
        .fetch_one(&self.pool)
        .await?;

        let content_types: Vec<(String,)> =
            sqlx::query_as("SELECT DISTINCT content_type FROM federated_memory WHERE agent_id = ?")
                .bind(agent_id)
                .fetch_all(&self.pool)
                .await?;

        let oldest: Option<(String,)> =
            sqlx::query_as("SELECT MIN(created_at) FROM federated_memory WHERE agent_id = ?")
                .bind(agent_id)
                .fetch_optional(&self.pool)
                .await?;

        let newest: Option<(String,)> =
            sqlx::query_as("SELECT MAX(created_at) FROM federated_memory WHERE agent_id = ?")
                .bind(agent_id)
                .fetch_optional(&self.pool)
                .await?;

        let parse_dt = |s: &str| {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .ok()
        };

        Ok(ContextManifest {
            agent_id: agent_id.to_string(),
            item_count: count as usize,
            total_tokens: total_tokens as usize,
            content_types: content_types.into_iter().map(|(ct,)| ct).collect(),
            oldest: oldest.and_then(|(s,)| parse_dt(&s)),
            newest: newest.and_then(|(s,)| parse_dt(&s)),
        })
    }

    /// Delete memory items older than `max_age_days`.
    pub async fn cleanup_old(&self, max_age_days: i64) -> Result<u64> {
        let cutoff = Utc::now() - chrono::Duration::days(max_age_days);

        let result = sqlx::query("DELETE FROM federated_memory WHERE created_at < ?")
            .bind(cutoff.to_rfc3339())
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected())
    }

    /// Total number of federated memory items.
    pub async fn count(&self) -> Result<u64> {
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM federated_memory")
            .fetch_one(&self.pool)
            .await?;
        Ok(count as u64)
    }
}

#[derive(sqlx::FromRow)]
struct RawRow {
    id: String,
    agent_id: String,
    session_id: String,
    content_type: String,
    content: Vec<u8>,
    summary: String,
    token_count: i64,
    policy_json: String,
    created_at: String,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    async fn create_test_service() -> FederatedMemoryService {
        static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
        let test_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        let db_path = std::path::PathBuf::from(format!(
            "file:arkavo_fed_mem_test_{timestamp}_{test_id}?mode=memory&cache=shared"
        ));

        FederatedMemoryService::new(&db_path).await.unwrap()
    }

    fn sample_item(agent: &str, session: &str, policy: MemoryPolicy) -> FederatedItem {
        FederatedItem {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: agent.to_string(),
            session_id: session.to_string(),
            content_type: "text/plain".to_string(),
            content: b"test content".to_vec(),
            summary: "test memory item".to_string(),
            token_count: 50,
            policy,
            created_at: Utc::now(),
        }
    }

    fn admin_policy() -> MemoryPolicy {
        MemoryPolicy {
            attributes: vec![MemoryAttribute {
                attribute: "https://arkavo.net/attr/role".to_string(),
                values: vec!["admin".to_string()],
            }],
        }
    }

    fn agent_policy(agent_id: &str) -> MemoryPolicy {
        MemoryPolicy {
            attributes: vec![MemoryAttribute {
                attribute: "https://arkavo.net/attr/agent-id".to_string(),
                values: vec![agent_id.to_string()],
            }],
        }
    }

    fn open_policy() -> MemoryPolicy {
        MemoryPolicy::default()
    }

    #[test]
    fn evaluate_empty_policy_permits() {
        let policy = MemoryPolicy::default();
        assert!(evaluate_entitlements(&[], &policy));
    }

    #[test]
    fn evaluate_matching_entitlement_permits() {
        let policy = admin_policy();
        let entitlements = vec!["https://arkavo.net/attr/role/value/admin".to_string()];
        assert!(evaluate_entitlements(&entitlements, &policy));
    }

    #[test]
    fn evaluate_missing_entitlement_denies() {
        let policy = admin_policy();
        let entitlements = vec!["https://arkavo.net/attr/role/value/user".to_string()];
        assert!(!evaluate_entitlements(&entitlements, &policy));
    }

    #[test]
    fn evaluate_empty_entitlements_denies_nonempty_policy() {
        let policy = admin_policy();
        assert!(!evaluate_entitlements(&[], &policy));
    }

    #[test]
    fn evaluate_multi_attribute_all_required() {
        let policy = MemoryPolicy {
            attributes: vec![
                MemoryAttribute {
                    attribute: "https://arkavo.net/attr/role".to_string(),
                    values: vec!["admin".to_string()],
                },
                MemoryAttribute {
                    attribute: "https://arkavo.net/attr/clearance".to_string(),
                    values: vec!["secret".to_string(), "top-secret".to_string()],
                },
            ],
        };

        // Has admin but wrong clearance
        let partial = vec![
            "https://arkavo.net/attr/role/value/admin".to_string(),
            "https://arkavo.net/attr/clearance/value/public".to_string(),
        ];
        assert!(!evaluate_entitlements(&partial, &policy));

        // Has both
        let full = vec![
            "https://arkavo.net/attr/role/value/admin".to_string(),
            "https://arkavo.net/attr/clearance/value/secret".to_string(),
        ];
        assert!(evaluate_entitlements(&full, &policy));
    }

    #[tokio::test]
    async fn store_and_query_open_policy() {
        let svc = create_test_service().await;
        let items = vec![sample_item("agent-1", "sess-1", open_policy())];

        svc.store_batch(&items).await.unwrap();

        let result = svc
            .query(&FederatedQuery {
                requester_id: "agent-2".to_string(),
                entitlements: vec![],
                session_filter: None,
                agent_filter: None,
                content_type_filter: None,
                since: None,
                limit: 100,
            })
            .await
            .unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.filtered_by_policy, 0);
    }

    #[tokio::test]
    async fn query_filters_by_policy() {
        let svc = create_test_service().await;
        let items = vec![
            sample_item("agent-1", "s1", admin_policy()),
            sample_item("agent-1", "s1", open_policy()),
            sample_item("agent-1", "s1", agent_policy("agent-3")),
        ];

        svc.store_batch(&items).await.unwrap();

        // Requester with no entitlements only sees open policy items
        let result = svc
            .query(&FederatedQuery {
                requester_id: "agent-2".to_string(),
                entitlements: vec![],
                session_filter: None,
                agent_filter: None,
                content_type_filter: None,
                since: None,
                limit: 100,
            })
            .await
            .unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.total_matched, 3);
        assert_eq!(result.filtered_by_policy, 2);
    }

    #[tokio::test]
    async fn query_with_admin_entitlement() {
        let svc = create_test_service().await;
        let items = vec![
            sample_item("agent-1", "s1", admin_policy()),
            sample_item("agent-1", "s1", open_policy()),
        ];

        svc.store_batch(&items).await.unwrap();

        let result = svc
            .query(&FederatedQuery {
                requester_id: "admin-agent".to_string(),
                entitlements: vec!["https://arkavo.net/attr/role/value/admin".to_string()],
                session_filter: None,
                agent_filter: None,
                content_type_filter: None,
                since: None,
                limit: 100,
            })
            .await
            .unwrap();

        assert_eq!(result.items.len(), 2);
        assert_eq!(result.filtered_by_policy, 0);
    }

    #[tokio::test]
    async fn query_with_agent_filter() {
        let svc = create_test_service().await;
        let items = vec![
            sample_item("agent-1", "s1", open_policy()),
            sample_item("agent-2", "s1", open_policy()),
            sample_item("agent-1", "s2", open_policy()),
        ];

        svc.store_batch(&items).await.unwrap();

        let result = svc
            .query(&FederatedQuery {
                requester_id: "requester".to_string(),
                entitlements: vec![],
                session_filter: None,
                agent_filter: Some("agent-1".to_string()),
                content_type_filter: None,
                since: None,
                limit: 100,
            })
            .await
            .unwrap();

        assert_eq!(result.items.len(), 2);
        assert!(result.items.iter().all(|i| i.agent_id == "agent-1"));
    }

    #[tokio::test]
    async fn query_with_session_filter() {
        let svc = create_test_service().await;
        let items = vec![
            sample_item("a1", "sess-a", open_policy()),
            sample_item("a1", "sess-b", open_policy()),
        ];

        svc.store_batch(&items).await.unwrap();

        let result = svc
            .query(&FederatedQuery {
                requester_id: "r".to_string(),
                entitlements: vec![],
                session_filter: Some("sess-a".to_string()),
                agent_filter: None,
                content_type_filter: None,
                since: None,
                limit: 100,
            })
            .await
            .unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].session_id, "sess-a");
    }

    #[tokio::test]
    async fn build_manifest_summarizes_agent_memory() {
        let svc = create_test_service().await;
        let items = vec![
            sample_item("agent-1", "s1", open_policy()),
            sample_item("agent-1", "s2", open_policy()),
            sample_item("agent-2", "s3", open_policy()),
        ];

        svc.store_batch(&items).await.unwrap();

        let manifest = svc.build_manifest("agent-1").await.unwrap();
        assert_eq!(manifest.item_count, 2);
        assert_eq!(manifest.total_tokens, 100); // 2 * 50
        assert!(manifest.content_types.contains(&"text/plain".to_string()));
    }

    #[tokio::test]
    async fn cleanup_old_items() {
        let svc = create_test_service().await;

        let mut old = sample_item("a1", "s1", open_policy());
        old.created_at = Utc::now() - chrono::Duration::days(40);

        let fresh = sample_item("a1", "s1", open_policy());

        svc.store_batch(&[old, fresh]).await.unwrap();

        let deleted = svc.cleanup_old(30).await.unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(svc.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn count_items() {
        let svc = create_test_service().await;
        assert_eq!(svc.count().await.unwrap(), 0);

        svc.store_batch(&[
            sample_item("a1", "s1", open_policy()),
            sample_item("a1", "s2", open_policy()),
        ])
        .await
        .unwrap();

        assert_eq!(svc.count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn upsert_replaces_existing_id() {
        let svc = create_test_service().await;

        let mut item = sample_item("a1", "s1", open_policy());
        let id = item.id.clone();
        item.summary = "original".to_string();
        svc.store_batch(&[item]).await.unwrap();

        let mut updated = sample_item("a1", "s1", open_policy());
        updated.id = id;
        updated.summary = "updated".to_string();
        svc.store_batch(&[updated]).await.unwrap();

        assert_eq!(svc.count().await.unwrap(), 1);

        let result = svc
            .query(&FederatedQuery {
                requester_id: "r".to_string(),
                entitlements: vec![],
                session_filter: None,
                agent_filter: None,
                content_type_filter: None,
                since: None,
                limit: 10,
            })
            .await
            .unwrap();

        assert_eq!(result.items[0].summary, "updated");
    }
}
