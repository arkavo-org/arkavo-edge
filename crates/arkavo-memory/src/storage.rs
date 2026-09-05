#[path = "replay_state.rs"]
mod replay_state;

use crate::embeddings::EmbeddingService;
use crate::error::{MemoryError, Result};
use crate::event_store::{EventStore, SerializedEvent, StoredEvent};
use crate::models::{AgentConversation, Memory, SearchResult};
use crate::workspace_config::WorkspaceConfig;
use hnsw_rs::prelude::*;
use sqlx::Row;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;
use uuid::Uuid;

// Embeddings are written as little-endian f32 bytes (see `store_with_replay_state`,
// which builds `embedding_blob` via `f.to_le_bytes()`). sqlx hands blobs back as a
// plain `Vec<u8>` with no alignment guarantee -- an empty `Vec<u8>` even has a
// dangling pointer with alignment 1 -- so casting the bytes to `&[f32]` in place
// (e.g. via `bytemuck::cast_slice`) can panic on unaligned input. Decoding
// f32-at-a-time with `from_le_bytes` copies out of the blob instead of reinterpreting
// it, so it works regardless of the blob's alignment. Any trailing bytes that don't
// fill a full 4-byte chunk are dropped rather than causing a panic.
fn decode_embedding(blob: &[u8]) -> Vec<f32> {
    blob.as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect()
}

// Lightweight struct for database queries that only need partial data
#[derive(sqlx::FromRow)]
struct MemoryRow {
    #[allow(dead_code)]
    id: String,
    category: Option<String>,
    embedding_blob: Vec<u8>, // Binary blob, more efficient than JSON
}

pub struct HnswConfig {
    pub max_nb_connection: usize,
    pub ef_construction: usize,
    pub max_elements: usize,
    pub max_hot_vectors: usize,
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self {
            max_nb_connection: 16,
            ef_construction: 200,
            max_elements: 100_000,
            max_hot_vectors: 10_000,
        }
    }
}

pub struct MemoryStorage {
    pool: SqlitePool,
    embeddings: Arc<RwLock<HashMap<Uuid, Vec<f32>>>>,
    index: Arc<RwLock<Hnsw<'static, f32, DistCosine>>>,
    id_mapping: Arc<RwLock<HashMap<usize, Uuid>>>,
    embedding_service: EmbeddingService,
    event_store: EventStore,
    config: HnswConfig,
    access_times: Arc<RwLock<HashMap<Uuid, Instant>>>,
    evicted_ids: Arc<RwLock<HashSet<Uuid>>>,
}

impl MemoryStorage {
    pub const CATEGORY_LEDGER_FRAGMENT: &'static str = "ledger_fragment";

    pub async fn new() -> Result<Self> {
        Self::with_config(HnswConfig::default()).await
    }

    /// Creates a new test instance with a unique database path.
    /// This should only be used in tests to ensure test isolation.
    pub async fn new_test() -> Result<Self> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

        let test_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("arkavo_test_{timestamp}_{test_id}.db"));

        Self::with_path(db_path, HnswConfig::default()).await
    }

    pub fn get_data_directory() -> Result<PathBuf> {
        // Check workspace config for absolute path first
        let config = WorkspaceConfig::get();
        let data_dir = if let Some(db_path) = &config.memory_db_path {
            // Use parent directory of the configured database path
            db_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from(".arkavo").join("memory_server"))
        } else {
            // Fallback to relative path (existing behavior)
            PathBuf::from(".arkavo").join("memory_server")
        };

        std::fs::create_dir_all(&data_dir)
            .map_err(|e| MemoryError::Storage(format!("Failed to create data directory: {e}")))?;

        Ok(data_dir)
    }

    pub async fn with_config(config: HnswConfig) -> Result<Self> {
        let data_dir = Self::get_data_directory()?;
        let db_path = data_dir.join("memories.db");
        Self::with_path(db_path, config).await
    }

    pub async fn with_path(db_path: PathBuf, config: HnswConfig) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                MemoryError::Storage(format!("Failed to create database directory: {e}"))
            })?;
        }

        // Use absolute path for SQLite
        let abs_db_path = std::fs::canonicalize(&db_path).unwrap_or(db_path);
        let database_url = format!("sqlite:{}?mode=rwc", abs_db_path.display());

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;

        Self::ensure_table_exists(&pool).await?;
        replay_state::ensure_table_exists(&pool).await?;

        let hnsw = Self::create_new_index(&config);
        let event_store = EventStore::new(pool.clone());

        let mut storage = Self {
            pool,
            embeddings: Arc::new(RwLock::new(HashMap::new())),
            index: Arc::new(RwLock::new(hnsw)),
            id_mapping: Arc::new(RwLock::new(HashMap::new())),
            embedding_service: EmbeddingService::new(),
            event_store,
            config,
            access_times: Arc::new(RwLock::new(HashMap::new())),
            evicted_ids: Arc::new(RwLock::new(HashSet::new())),
        };

        storage.load_embeddings_from_db().await?;

        Ok(storage)
    }

    async fn ensure_table_exists(pool: &SqlitePool) -> Result<()> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                metadata TEXT,
                category TEXT,
                embedding_blob BLOB NOT NULL,
                created_at TIMESTAMP NOT NULL,
                updated_at TIMESTAMP NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_memories_created_at ON memories(created_at)
            "#,
        )
        .execute(pool)
        .await?;

        // FTS5 virtual table for cold tier text search
        sqlx::query(
            r#"
            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                content,
                content='memories',
                content_rowid='rowid'
            )
            "#,
        )
        .execute(pool)
        .await?;

        // Trigger to keep FTS in sync on insert
        sqlx::query(
            r#"
            CREATE TRIGGER IF NOT EXISTS memories_fts_insert AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END
            "#,
        )
        .execute(pool)
        .await?;

        // Trigger to keep FTS in sync on update
        sqlx::query(
            r#"
            CREATE TRIGGER IF NOT EXISTS memories_fts_update AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END
            "#,
        )
        .execute(pool)
        .await?;

        // Trigger to keep FTS in sync on delete
        sqlx::query(
            r#"
            CREATE TRIGGER IF NOT EXISTS memories_fts_delete AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
            END
            "#,
        )
        .execute(pool)
        .await?;

        // Create events table for event store
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                timestamp TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload BLOB NOT NULL,
                schema_version TEXT NOT NULL,
                created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_events_session ON events(session_id)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_events_agent ON events(agent_id)
            "#,
        )
        .execute(pool)
        .await?;

        // Create agent conversations table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS agent_conversations (
                id TEXT PRIMARY KEY,
                from_agent_id TEXT NOT NULL,
                to_agent_id TEXT NOT NULL,
                query TEXT NOT NULL,
                response TEXT NOT NULL,
                confidence REAL NOT NULL,
                domain TEXT,
                created_at TIMESTAMP NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_conversations_agents 
            ON agent_conversations(from_agent_id, to_agent_id)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_conversations_domain 
            ON agent_conversations(domain)
            "#,
        )
        .execute(pool)
        .await?;

        // Create events table for debugging
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS events (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                timestamp TIMESTAMP NOT NULL,
                agent_id TEXT NOT NULL,
                event_type TEXT NOT NULL,
                payload BLOB NOT NULL,
                schema_version TEXT NOT NULL,
                created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                UNIQUE(session_id, sequence)
            )
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_events_session 
            ON events(session_id, sequence)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_events_timestamp 
            ON events(timestamp)
            "#,
        )
        .execute(pool)
        .await?;

        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_events_created 
            ON events(created_at)
            "#,
        )
        .execute(pool)
        .await?;

        Ok(())
    }

    fn create_new_index(config: &HnswConfig) -> Hnsw<'static, f32, DistCosine> {
        Hnsw::<f32, DistCosine>::new(
            config.max_nb_connection,
            config.max_elements,
            16,
            config.ef_construction,
            DistCosine,
        )
    }

    async fn load_embeddings_from_db(&mut self) -> Result<()> {
        let max_hot = self.config.max_hot_vectors;

        // Load only most recent memories up to hot tier limit
        let rows = sqlx::query(
            "SELECT id, embedding_blob FROM memories
             WHERE embedding_blob IS NOT NULL AND length(embedding_blob) > 0
             ORDER BY updated_at DESC
             LIMIT ?",
        )
        .bind(max_hot as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut parsed: Vec<(Uuid, Vec<f32>)> = Vec::with_capacity(rows.len());
        for row in rows {
            let id_str: String = row.get("id");
            let embedding_blob: Vec<u8> = row.get("embedding_blob");
            let id = Uuid::parse_str(&id_str)
                .map_err(|e| MemoryError::Storage(format!("Invalid UUID: {e}")))?;
            let embedding: Vec<f32> = decode_embedding(&embedding_blob);
            parsed.push((id, embedding));
        }

        let embeddings_clone = Arc::clone(&self.embeddings);
        let index_clone = Arc::clone(&self.index);
        let id_mapping_clone = Arc::clone(&self.id_mapping);
        let access_times_clone = Arc::clone(&self.access_times);

        tokio::task::spawn_blocking(move || {
            let mut embeddings = embeddings_clone.write().unwrap();
            let index = index_clone.write().unwrap();
            let mut id_mapping = id_mapping_clone.write().unwrap();
            let mut access_times = access_times_clone.write().unwrap();
            let now = Instant::now();

            for (idx, (id, embedding)) in parsed.into_iter().enumerate() {
                if !embedding.is_empty() {
                    embeddings.insert(id, embedding.clone());
                    id_mapping.insert(idx, id);
                    index.insert((&embedding, idx));
                    access_times.insert(id, now);
                }
            }
        })
        .await
        .map_err(|e| MemoryError::Storage(format!("Index load failed: {e}")))?;

        Ok(())
    }

    fn get_next_index(&self) -> usize {
        let id_mapping = self.id_mapping.read().unwrap();
        id_mapping.len()
    }

    fn soft_evict_oldest(&self) {
        let evicted = self.evicted_ids.read().unwrap();
        let oldest_id = {
            let access_times = self.access_times.read().unwrap();
            access_times
                .iter()
                .filter(|(id, _)| !evicted.contains(id))
                .min_by_key(|(_, time)| *time)
                .map(|(id, _)| *id)
        };
        drop(evicted);

        if let Some(id) = oldest_id {
            let mut evicted = self.evicted_ids.write().unwrap();
            evicted.insert(id);
        }
    }

    async fn maybe_rebuild_index(&self) -> Result<()> {
        let (evicted_count, total_count) = {
            let evicted = self.evicted_ids.read().unwrap();
            let embeddings = self.embeddings.read().unwrap();
            (evicted.len(), embeddings.len())
        };

        // Rebuild when >20% evicted
        if total_count > 0 && evicted_count * 5 > total_count {
            self.rebuild_index().await?;
        }
        Ok(())
    }

    async fn rebuild_index(&self) -> Result<()> {
        let (active_embeddings, active_ids): (Vec<_>, Vec<_>) = {
            let embeddings = self.embeddings.read().unwrap();
            let evicted = self.evicted_ids.read().unwrap();
            embeddings
                .iter()
                .filter(|(id, _)| !evicted.contains(id))
                .map(|(id, emb)| (emb.clone(), *id))
                .unzip()
        };

        let max_elements = self.config.max_elements;
        let max_nb_connection = self.config.max_nb_connection;
        let ef_construction = self.config.ef_construction;

        // Rebuild in blocking task
        let result = tokio::task::spawn_blocking(move || {
            let hnsw = Hnsw::<f32, DistCosine>::new(
                max_nb_connection,
                max_elements,
                16,
                ef_construction,
                DistCosine,
            );
            let mut new_mapping = HashMap::new();
            for (idx, (embedding, id)) in active_embeddings.into_iter().zip(active_ids).enumerate()
            {
                hnsw.insert((&embedding, idx));
                new_mapping.insert(idx, id);
            }
            (hnsw, new_mapping)
        })
        .await
        .map_err(|e| MemoryError::Storage(format!("Rebuild failed: {e}")))?;

        // Swap in new index
        {
            *self.index.write().unwrap() = result.0;
            *self.id_mapping.write().unwrap() = result.1.clone();
            self.evicted_ids.write().unwrap().clear();
            // Also clean up embeddings HashMap and access_times
            let active_ids: HashSet<_> = result.1.values().copied().collect();
            self.embeddings
                .write()
                .unwrap()
                .retain(|id, _| active_ids.contains(id));
            self.access_times
                .write()
                .unwrap()
                .retain(|id, _| active_ids.contains(id));
        }

        Ok(())
    }

    pub async fn store_ledger_fragment(
        &self,
        id: Uuid,
        content: String,
        summary: String,
        source: String,
    ) -> Result<()> {
        let embedding = self.embedding_service.generate_embedding(&content).await?;

        // Metadata as JSON
        let metadata = serde_json::json!({
            "summary": summary,
            "source": source,
            "type": "fragment"
        });

        let memory = Memory {
            id,
            content,
            metadata: Some(metadata),
            category: Some(Self::CATEGORY_LEDGER_FRAGMENT.to_string()),
            embedding,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.store(memory).await
    }

    pub async fn get_ledger_fragment(&self, id: Uuid) -> Result<Memory> {
        self.get(id).await
    }

    pub async fn store(&self, memory: Memory) -> Result<()> {
        self.store_with_replay_state(memory, None).await
    }

    /// Persist a public memory and its private provider state atomically.
    /// State is never included in embeddings, search results, or public metadata.
    pub async fn store_with_replay_state(
        &self,
        memory: Memory,
        replay_state: Option<&[u8]>,
    ) -> Result<()> {
        let id_str = memory.id.to_string();
        let embedding_blob: Vec<u8> = memory
            .embedding
            .iter()
            .flat_map(|&f| f.to_le_bytes())
            .collect();
        let metadata_json = memory
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            r#"
            INSERT INTO memories (id, content, metadata, category, embedding_blob, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&id_str)
        .bind(&memory.content)
        .bind(&metadata_json)
        .bind(&memory.category)
        .bind(&embedding_blob)
        .bind(memory.created_at.to_rfc3339())
        .bind(memory.updated_at.to_rfc3339())
        .execute(&mut *transaction)
        .await?;

        if let Some(state) = replay_state {
            sqlx::query("INSERT INTO conversation_replay_state (memory_id, state) VALUES (?, ?)")
                .bind(&id_str)
                .bind(state)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;

        // Only update indexes if embedding is not empty
        if !memory.embedding.is_empty() {
            // Check if eviction needed (count non-evicted items)
            let active_count = {
                let embeddings = self.embeddings.read().unwrap();
                let evicted = self.evicted_ids.read().unwrap();
                embeddings.len() - evicted.len()
            };

            if active_count >= self.config.max_hot_vectors {
                self.soft_evict_oldest();
            }

            let next_idx = self.get_next_index();

            {
                let mut embeddings = self.embeddings.write().unwrap();
                embeddings.insert(memory.id, memory.embedding.clone());
            }

            {
                let mut id_mapping = self.id_mapping.write().unwrap();
                id_mapping.insert(next_idx, memory.id);
            }

            let index_clone = Arc::clone(&self.index);
            let embedding_data = memory.embedding.clone();
            tokio::task::spawn_blocking(move || {
                let index = index_clone.write().unwrap();
                index.insert((&embedding_data, next_idx));
            })
            .await
            .map_err(|e| MemoryError::Storage(format!("Index insert failed: {e}")))?;

            // Track access time
            {
                let mut access_times = self.access_times.write().unwrap();
                access_times.insert(memory.id, Instant::now());
            }
        }

        Ok(())
    }

    pub async fn get(&self, id: Uuid) -> Result<Memory> {
        let id_str = id.to_string();
        let row = sqlx::query("SELECT * FROM memories WHERE id = ?")
            .bind(&id_str)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(MemoryError::NotFound)?;

        let embedding_blob: Vec<u8> = row.get("embedding_blob");
        let embedding: Vec<f32> = decode_embedding(&embedding_blob);

        let metadata_str: Option<String> = row.get("metadata");
        let metadata = metadata_str
            .as_ref()
            .map(|m| serde_json::from_str(m))
            .transpose()?;

        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");

        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| MemoryError::Storage(format!("Invalid created_at timestamp: {e}")))?
            .with_timezone(&chrono::Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| MemoryError::Storage(format!("Invalid updated_at timestamp: {e}")))?
            .with_timezone(&chrono::Utc);

        Ok(Memory {
            id,
            content: row.get("content"),
            metadata,
            category: row.get("category"),
            embedding,
            created_at,
            updated_at,
        })
    }

    pub async fn delete(&self, id: Uuid) -> Result<()> {
        let id_str = id.to_string();
        let rows_affected = sqlx::query("DELETE FROM memories WHERE id = ?")
            .bind(&id_str)
            .execute(&self.pool)
            .await?
            .rows_affected();

        if rows_affected == 0 {
            return Err(MemoryError::NotFound);
        }

        self.embeddings.write().unwrap().remove(&id);
        self.access_times.write().unwrap().remove(&id);
        self.evicted_ids.write().unwrap().remove(&id);
        self.id_mapping
            .write()
            .unwrap()
            .retain(|_, mapped| *mapped != id);

        Ok(())
    }

    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        category: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        // 1. Vector search on hot tier (skip evicted)
        let mut results = self.search_hot(query, limit, category).await?;

        // 2. If insufficient results, search cold tier via FTS5
        if results.len() < limit {
            let hot_ids: HashSet<Uuid> = results.iter().map(|r| r.memory.id).collect();
            let cold_results = self
                .search_cold_fts(query, limit - results.len(), category, &hot_ids)
                .await?;
            results.extend(cold_results);
        }

        // 3. Maybe rebuild index if too many evicted
        self.maybe_rebuild_index().await?;

        Ok(results)
    }

    /// Every memory in a category, newest first.
    ///
    /// Exact, not nearest-neighbour. Listing is not a search: conversation
    /// sessions all share a placeholder zero-vector, so HNSW can drop one.
    pub async fn list_by_category(&self, category: &str, limit: usize) -> Result<Vec<Memory>> {
        let rows = sqlx::query(
            "SELECT id, content, metadata, category, embedding_blob, created_at, updated_at
             FROM memories
             WHERE category = ?
             ORDER BY updated_at DESC
             LIMIT ?",
        )
        .bind(category)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let id_str: String = row.get("id");
            let id = Uuid::parse_str(&id_str)
                .map_err(|e| MemoryError::Storage(format!("Invalid UUID: {e}")))?;

            let embedding_blob: Vec<u8> = row.get("embedding_blob");
            let embedding: Vec<f32> = decode_embedding(&embedding_blob);

            let metadata_str: Option<String> = row.get("metadata");
            let metadata = metadata_str
                .as_ref()
                .map(|m| serde_json::from_str(m))
                .transpose()?;

            let created_at_str: String = row.get("created_at");
            let updated_at_str: String = row.get("updated_at");
            let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
                .map_err(|e| MemoryError::Storage(format!("Invalid created_at timestamp: {e}")))?
                .with_timezone(&chrono::Utc);
            let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                .map_err(|e| MemoryError::Storage(format!("Invalid updated_at timestamp: {e}")))?
                .with_timezone(&chrono::Utc);

            out.push(Memory {
                id,
                content: row.get("content"),
                metadata,
                category: row.get("category"),
                embedding,
                created_at,
                updated_at,
            });
        }
        Ok(out)
    }

    async fn search_hot(
        &self,
        query: &str,
        limit: usize,
        category: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let query_embedding = self.embedding_service.generate_embedding(query).await?;

        let index_clone = Arc::clone(&self.index);
        let k = limit * 2;
        let ef_search = limit * 10;
        let neighbors = tokio::task::spawn_blocking(move || {
            let index = index_clone.read().unwrap();
            index.search(&query_embedding, k, ef_search)
        })
        .await
        .map_err(|e| MemoryError::Storage(format!("Search task failed: {e}")))?;

        // Map to UUIDs, filtering evicted
        let mut neighbor_ids = Vec::new();
        {
            let evicted = self.evicted_ids.read().unwrap();
            let id_mapping = self.id_mapping.read().unwrap();
            for neighbor in &neighbors {
                if let Some(&id) = id_mapping.get(&neighbor.d_id)
                    && !evicted.contains(&id)
                {
                    neighbor_ids.push((id, neighbor.distance));
                }
            }
        }

        let mut results = Vec::new();

        for (id, distance) in neighbor_ids {
            let memory = match self.get(id).await {
                Ok(m) => m,
                Err(_) => continue,
            };

            if let Some(cat) = category
                && memory.category.as_ref() != Some(&cat.to_string())
            {
                continue;
            }

            let score = 1.0 - distance;

            // Update access time
            {
                let mut access_times = self.access_times.write().unwrap();
                access_times.insert(id, Instant::now());
            }

            results.push(SearchResult { memory, score });

            if results.len() >= limit {
                break;
            }
        }

        Ok(results)
    }

    fn sanitize_fts5_word(word: &str) -> String {
        word.chars()
            .filter(|c| !matches!(c, '*' | '-' | '(' | ')' | '{' | '}'))
            .collect()
    }

    async fn search_cold_fts(
        &self,
        query: &str,
        limit: usize,
        category: Option<&str>,
        exclude_ids: &HashSet<Uuid>,
    ) -> Result<Vec<SearchResult>> {
        // Build FTS5 query - sanitize operators and escape quotes
        let fts_query = query
            .split_whitespace()
            .map(|word| {
                let sanitized = Self::sanitize_fts5_word(word);
                format!("\"{}\"", sanitized.replace('"', "\"\""))
            })
            .filter(|word| word != "\"\"")
            .collect::<Vec<_>>()
            .join(" OR ");

        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let rows = if let Some(cat) = category {
            sqlx::query(
                "SELECT m.id, m.content, m.metadata, m.category, m.embedding_blob,
                        m.created_at, m.updated_at, bm25(memories_fts) as rank
                 FROM memories m
                 JOIN memories_fts fts ON m.rowid = fts.rowid
                 WHERE memories_fts MATCH ? AND m.category = ?
                 ORDER BY rank
                 LIMIT ?",
            )
            .bind(&fts_query)
            .bind(cat)
            .bind((limit * 2) as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT m.id, m.content, m.metadata, m.category, m.embedding_blob,
                        m.created_at, m.updated_at, bm25(memories_fts) as rank
                 FROM memories m
                 JOIN memories_fts fts ON m.rowid = fts.rowid
                 WHERE memories_fts MATCH ?
                 ORDER BY rank
                 LIMIT ?",
            )
            .bind(&fts_query)
            .bind((limit * 2) as i64)
            .fetch_all(&self.pool)
            .await?
        };

        let mut results = Vec::new();

        for row in rows {
            let id_str: String = row.get("id");
            let id = Uuid::parse_str(&id_str)
                .map_err(|e| MemoryError::Storage(format!("Invalid UUID: {e}")))?;

            // Skip if already in hot results
            if exclude_ids.contains(&id) {
                continue;
            }

            let embedding_blob: Vec<u8> = row.get("embedding_blob");
            let embedding: Vec<f32> = decode_embedding(&embedding_blob);

            let metadata_str: Option<String> = row.get("metadata");
            let metadata = metadata_str
                .as_ref()
                .map(|m| serde_json::from_str(m))
                .transpose()?;

            let created_at_str: String = row.get("created_at");
            let updated_at_str: String = row.get("updated_at");

            let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
                .map_err(|e| MemoryError::Storage(format!("Invalid created_at timestamp: {e}")))?
                .with_timezone(&chrono::Utc);
            let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
                .map_err(|e| MemoryError::Storage(format!("Invalid updated_at timestamp: {e}")))?
                .with_timezone(&chrono::Utc);

            let rank: f64 = row.get("rank");
            // BM25 returns negative values, more negative = better match
            // Normalize to 0-1 range (approximate)
            let score = (1.0 / (1.0 - rank)).clamp(0.0, 1.0) as f32;

            let memory = Memory {
                id,
                content: row.get("content"),
                metadata,
                category: row.get("category"),
                embedding,
                created_at,
                updated_at,
            };

            // Promote cold memory to hot tier
            self.promote_to_hot(&memory).await?;

            results.push(SearchResult { memory, score });

            if results.len() >= limit {
                break;
            }
        }

        Ok(results)
    }

    async fn promote_to_hot(&self, memory: &Memory) -> Result<()> {
        if memory.embedding.is_empty() {
            return Ok(());
        }

        // Check if already in hot tier
        {
            let embeddings = self.embeddings.read().unwrap();
            if embeddings.contains_key(&memory.id) {
                // Just update access time
                let mut access_times = self.access_times.write().unwrap();
                access_times.insert(memory.id, Instant::now());
                return Ok(());
            }
        }

        // Evict if needed
        let active_count = {
            let embeddings = self.embeddings.read().unwrap();
            let evicted = self.evicted_ids.read().unwrap();
            embeddings.len() - evicted.len()
        };

        if active_count >= self.config.max_hot_vectors {
            self.soft_evict_oldest();
        }

        // Add to hot tier
        let next_idx = self.get_next_index();

        {
            let mut embeddings = self.embeddings.write().unwrap();
            embeddings.insert(memory.id, memory.embedding.clone());
        }

        {
            let mut id_mapping = self.id_mapping.write().unwrap();
            id_mapping.insert(next_idx, memory.id);
        }

        let index_clone = Arc::clone(&self.index);
        let embedding_data = memory.embedding.clone();
        tokio::task::spawn_blocking(move || {
            let index = index_clone.write().unwrap();
            index.insert((&embedding_data, next_idx));
        })
        .await
        .map_err(|e| MemoryError::Storage(format!("Promote insert failed: {e}")))?;

        // Track access time
        {
            let mut access_times = self.access_times.write().unwrap();
            access_times.insert(memory.id, Instant::now());
        }

        Ok(())
    }

    pub async fn categorize(&self, content: &str) -> Result<(String, f32)> {
        let embedding = self.embedding_service.generate_embedding(content).await?;

        // Get all memories with categories using the lightweight MemoryRow struct
        let rows: Vec<MemoryRow> = sqlx::query_as::<_, MemoryRow>(
            "SELECT id, category, embedding_blob
             FROM memories
             WHERE category IS NOT NULL AND embedding_blob IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Ok(("uncategorized".to_string(), 1.0));
        }

        let mut best_category = "uncategorized".to_string();
        let mut best_score = 0.0;

        // Find the memory with the highest similarity
        for row in rows {
            if let Some(category) = row.category {
                // Convert binary blob to f32 vector
                let mem_embedding: Vec<f32> = decode_embedding(&row.embedding_blob);
                let score = EmbeddingService::cosine_similarity(&embedding, &mem_embedding);

                if score > best_score {
                    best_score = score;
                    best_category = category;
                }
            }
        }

        // Use a lower threshold to be more accepting of similar content
        let threshold = 0.3; // Lower threshold from 0.4 to 0.3
        if best_score > threshold {
            Ok((best_category, best_score))
        } else {
            Ok(("uncategorized".to_string(), best_score))
        }
    }

    /// Store an agent conversation
    pub async fn store_conversation(&self, conversation: AgentConversation) -> Result<()> {
        let id_str = conversation.id.to_string();

        sqlx::query(
            r#"
            INSERT INTO agent_conversations 
            (id, from_agent_id, to_agent_id, query, response, confidence, domain, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&id_str)
        .bind(&conversation.from_agent_id)
        .bind(&conversation.to_agent_id)
        .bind(&conversation.query)
        .bind(&conversation.response)
        .bind(conversation.confidence)
        .bind(&conversation.domain)
        .bind(conversation.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Get recent conversations between agents
    pub async fn get_agent_conversations(
        &self,
        from_agent_id: Option<&str>,
        to_agent_id: Option<&str>,
        domain: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AgentConversation>> {
        let mut query = String::from("SELECT * FROM agent_conversations WHERE 1=1");
        let mut params = Vec::new();

        if let Some(from_id) = from_agent_id {
            query.push_str(" AND from_agent_id = ?");
            params.push(from_id.to_string());
        }

        if let Some(to_id) = to_agent_id {
            query.push_str(" AND to_agent_id = ?");
            params.push(to_id.to_string());
        }

        if let Some(d) = domain {
            query.push_str(" AND domain = ?");
            params.push(d.to_string());
        }

        query.push_str(" ORDER BY created_at DESC LIMIT ?");

        let mut sql_query = sqlx::query(&query);
        for param in params {
            sql_query = sql_query.bind(param);
        }
        sql_query = sql_query.bind(limit as i64);

        let rows = sql_query.fetch_all(&self.pool).await?;

        let mut conversations = Vec::new();
        for row in rows {
            let id_str: String = row.get("id");
            let created_at_str: String = row.get("created_at");

            let conversation = AgentConversation {
                id: Uuid::parse_str(&id_str)
                    .map_err(|e| MemoryError::Storage(format!("Invalid UUID: {e}")))?,
                from_agent_id: row.get("from_agent_id"),
                to_agent_id: row.get("to_agent_id"),
                query: row.get("query"),
                response: row.get("response"),
                confidence: row.get("confidence"),
                domain: row.get("domain"),
                created_at: chrono::DateTime::parse_from_rfc3339(&created_at_str)
                    .map_err(|e| MemoryError::Storage(format!("Invalid timestamp: {e}")))?
                    .with_timezone(&chrono::Utc),
            };
            conversations.push(conversation);
        }

        Ok(conversations)
    }

    /// Search for agent knowledge by domain
    pub async fn search_by_domain(
        &self,
        domain: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Use the existing search method with category filter
        self.search(query, limit, Some(domain)).await
    }

    // Event Store Methods

    pub async fn store_events(&self, events: Vec<SerializedEvent>) -> Result<()> {
        self.event_store.store_events(events).await
    }

    pub async fn get_session_events(
        &self,
        session_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<StoredEvent>> {
        self.event_store.get_session_events(session_id, limit).await
    }

    pub async fn get_recent_events(
        &self,
        agent_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>> {
        self.event_store.get_recent_events(agent_id, limit).await
    }

    pub async fn get_events_since(
        &self,
        since: chrono::DateTime<chrono::Utc>,
        agent_id: Option<&str>,
    ) -> Result<Vec<StoredEvent>> {
        self.event_store.get_events_since(since, agent_id).await
    }

    pub async fn get_event_stats(&self) -> Result<(u64, u64)> {
        let session_count = self.event_store.get_session_count().await?;
        let event_count = self.event_store.get_total_event_count().await?;
        Ok((session_count, event_count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_fts5_word_removes_operators() {
        assert_eq!(MemoryStorage::sanitize_fts5_word("hello*"), "hello");
        assert_eq!(MemoryStorage::sanitize_fts5_word("-exclude"), "exclude");
        assert_eq!(MemoryStorage::sanitize_fts5_word("(group)"), "group");
        assert_eq!(MemoryStorage::sanitize_fts5_word("{column}"), "column");
        assert_eq!(MemoryStorage::sanitize_fts5_word("test-word"), "testword");
    }

    #[test]
    fn test_sanitize_fts5_word_preserves_normal_text() {
        assert_eq!(MemoryStorage::sanitize_fts5_word("hello"), "hello");
        assert_eq!(MemoryStorage::sanitize_fts5_word("test123"), "test123");
        assert_eq!(
            MemoryStorage::sanitize_fts5_word("unicode_text"),
            "unicode_text"
        );
    }

    #[test]
    fn test_sanitize_fts5_word_handles_complex_input() {
        assert_eq!(
            MemoryStorage::sanitize_fts5_word("hello*world-test"),
            "helloworldtest"
        );
        assert_eq!(MemoryStorage::sanitize_fts5_word("***"), "");
        assert_eq!(MemoryStorage::sanitize_fts5_word("a(b)c{d}e"), "abcde");
    }

    #[test]
    fn test_sanitize_fts5_word_preserves_quotes_for_caller() {
        assert_eq!(
            MemoryStorage::sanitize_fts5_word("hello\"world"),
            "hello\"world"
        );
    }

    // Regression test for a Linux-only panic: `bytemuck::cast_slice::<u8, f32>`
    // requires the input slice to be 4-byte aligned, but a `Vec<u8>` returned by
    // sqlx carries no such guarantee. Slicing off a leading byte here reproduces
    // an unaligned blob the way sqlx can hand one back; `decode_embedding` must
    // decode it without relying on the slice's address.
    #[test]
    fn test_decode_embedding_handles_misaligned_blob() {
        let values: [f32; 3] = [1.0, -2.5, 3.25];
        let mut bytes = vec![0u8]; // leading pad byte forces misalignment below
        for v in values {
            bytes.extend_from_slice(&v.to_le_bytes());
        }

        let misaligned = &bytes[1..];
        assert_ne!(
            misaligned
                .as_ptr()
                .align_offset(std::mem::align_of::<f32>()),
            0,
            "test setup should produce a misaligned slice"
        );

        assert_eq!(decode_embedding(misaligned), values.to_vec());
    }

    #[test]
    fn test_decode_embedding_empty_blob_yields_empty_vec() {
        assert_eq!(decode_embedding(&[]), Vec::<f32>::new());
    }

    #[test]
    fn test_decode_embedding_ignores_trailing_partial_chunk() {
        let mut bytes = 1.5f32.to_le_bytes().to_vec();
        bytes.extend_from_slice(&[0xAA, 0xBB, 0xCC]); // 3 stray bytes, not a full f32
        assert_eq!(decode_embedding(&bytes), vec![1.5]);
    }
}
