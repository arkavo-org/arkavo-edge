//! Provider continuation state is deliberately absent from searchable memories.
//! Only session restoration accesses these bytes; memory tools and exports query
//! the public memories table and cannot accidentally render opaque model state.

use super::MemoryStorage;
use crate::error::Result;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool};
use std::collections::HashMap;
use uuid::Uuid;

pub(super) async fn ensure_table_exists(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS conversation_replay_state (
            memory_id TEXT PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
            state BLOB NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

impl MemoryStorage {
    /// Retrieve private continuation bytes by their public conversation record ID.
    /// This API is intentionally separate from get/search/list memory results.
    pub async fn load_replay_state(&self, memory_id: Uuid) -> Result<Option<Vec<u8>>> {
        Ok(
            sqlx::query_scalar("SELECT state FROM conversation_replay_state WHERE memory_id = ?")
                .bind(memory_id.to_string())
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Retrieve the continuation bytes for a whole context window at once.
    ///
    /// Rebuilding a window is a per-message loop only in the caller; issuing one
    /// statement per message turns restoring a session into an N+1 query.
    /// Records without private state are simply absent from the result.
    pub async fn load_replay_states(&self, memory_ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<u8>>> {
        if memory_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let mut builder: QueryBuilder<Sqlite> = QueryBuilder::new(
            "SELECT memory_id, state FROM conversation_replay_state WHERE memory_id IN (",
        );
        let mut separated = builder.separated(", ");
        for memory_id in memory_ids {
            separated.push_bind(memory_id.to_string());
        }
        separated.push_unseparated(")");

        let rows = builder.build().fetch_all(&self.pool).await?;
        let mut states = HashMap::with_capacity(rows.len());
        for row in rows {
            let memory_id: String = row.try_get("memory_id")?;
            let Ok(memory_id) = memory_id.parse::<Uuid>() else {
                continue;
            };
            states.insert(memory_id, row.try_get::<Vec<u8>, _>("state")?);
        }
        Ok(states)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Memory;

    #[tokio::test]
    async fn private_state_is_not_searchable_and_follows_memory_deletion() {
        let storage = MemoryStorage::new_test().await.unwrap();
        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let memory = Memory {
            id,
            content: "visible conversation".into(),
            metadata: None,
            category: Some("conversation".into()),
            embedding: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        storage
            .store_with_replay_state(memory, Some(b"opaque-replay-canary"))
            .await
            .unwrap();
        assert_eq!(
            storage.load_replay_state(id).await.unwrap().unwrap(),
            b"opaque-replay-canary"
        );
        let visible = storage.get(id).await.unwrap();
        assert_eq!(visible.content, "visible conversation");
        let listed = storage.list_by_category("conversation", 10).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(
            !serde_json::to_string(&listed)
                .unwrap()
                .contains("opaque-replay-canary")
        );
        let found = storage
            .search("opaque-replay-canary", 10, None)
            .await
            .unwrap();
        assert!(
            found
                .iter()
                .all(|r| !r.memory.content.contains("opaque-replay-canary"))
        );
        storage.delete(id).await.unwrap();
        assert!(storage.load_replay_state(id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_whole_window_of_replay_state_loads_in_one_query() {
        let storage = MemoryStorage::new_test().await.unwrap();
        let now = chrono::Utc::now();
        let mut with_state = Vec::new();
        for index in 0..12u8 {
            let id = Uuid::new_v4();
            let memory = Memory {
                id,
                content: format!("message {index}"),
                metadata: None,
                category: Some("conversation".into()),
                embedding: Vec::new(),
                created_at: now,
                updated_at: now,
            };
            // Only even records carry continuation bytes.
            let state = (index % 2 == 0).then(|| format!("state-{index}").into_bytes());
            storage
                .store_with_replay_state(memory, state.as_deref())
                .await
                .unwrap();
            with_state.push((id, state));
        }

        let ids: Vec<Uuid> = with_state.iter().map(|(id, _)| *id).collect();
        let states = storage.load_replay_states(&ids).await.unwrap();
        assert_eq!(states.len(), 6);
        for (id, expected) in &with_state {
            assert_eq!(states.get(id), expected.as_ref());
        }
        assert!(
            storage
                .load_replay_states(&[Uuid::new_v4()])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(storage.load_replay_states(&[]).await.unwrap().is_empty());
    }
}
