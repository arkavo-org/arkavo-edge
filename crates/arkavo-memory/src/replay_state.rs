//! Provider continuation state is deliberately absent from searchable memories.
//! Only session restoration accesses these bytes; memory tools and exports query
//! the public memories table and cannot accidentally render opaque model state.

use super::MemoryStorage;
use crate::error::Result;
use sqlx::SqlitePool;
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
}
