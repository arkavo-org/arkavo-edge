use crate::embeddings::EmbeddingService;
use crate::error::{MemoryError, Result};
use crate::event_store::{EventStore, SerializedEvent, StoredEvent};
use crate::models::{AgentConversation, Memory, SearchResult};
use hnsw_rs::prelude::*;
use sqlx::Row;
use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

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
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self {
            max_nb_connection: 16,
            ef_construction: 200,
            max_elements: 100_000,
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
    #[allow(dead_code)]
    config: HnswConfig,
}

impl MemoryStorage {
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
        let data_dir = PathBuf::from(".arkavo").join("memory_server");

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
        let rows = sqlx::query("SELECT id, embedding_blob FROM memories")
            .fetch_all(&self.pool)
            .await?;

        let mut embeddings = self.embeddings.write().unwrap();
        let index = self.index.write().unwrap();
        let mut id_mapping = self.id_mapping.write().unwrap();

        for (idx, row) in rows.iter().enumerate() {
            let id_str: String = row.get("id");
            let embedding_blob: Vec<u8> = row.get("embedding_blob");

            let id = Uuid::parse_str(&id_str)
                .map_err(|e| MemoryError::Storage(format!("Invalid UUID: {e}")))?;
            let embedding: Vec<f32> = embedding_blob
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();

            // Skip empty embeddings (e.g., from config entries)
            if !embedding.is_empty() {
                embeddings.insert(id, embedding.clone());
                id_mapping.insert(idx, id);

                let mut point_data = Vec::with_capacity(embedding.len());
                point_data.extend_from_slice(&embedding);
                index.insert((&point_data, idx));
            }
        }

        Ok(())
    }

    fn get_next_index(&self) -> usize {
        let id_mapping = self.id_mapping.read().unwrap();
        id_mapping.len()
    }

    pub async fn store(&self, memory: Memory) -> Result<()> {
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
        .execute(&self.pool)
        .await?;

        // Only update indexes if embedding is not empty
        if !memory.embedding.is_empty() {
            let next_idx = self.get_next_index();

            {
                let mut embeddings = self.embeddings.write().unwrap();
                embeddings.insert(memory.id, memory.embedding.clone());
            }

            {
                let mut id_mapping = self.id_mapping.write().unwrap();
                id_mapping.insert(next_idx, memory.id);
            }

            {
                let index = self.index.write().unwrap();
                let mut point_data = Vec::with_capacity(memory.embedding.len());
                point_data.extend_from_slice(&memory.embedding);
                index.insert((&point_data, next_idx));
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
        let embedding: Vec<f32> = bytemuck::cast_slice(&embedding_blob).to_vec();

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

    pub async fn search(
        &self,
        query: &str,
        limit: usize,
        category: Option<&str>,
    ) -> Result<Vec<SearchResult>> {
        let query_embedding = self.embedding_service.generate_embedding(query).await?;

        let neighbors = {
            let index = self.index.read().unwrap();
            let ef_search = limit * 10;
            index.search(&query_embedding, limit * 2, ef_search)
        };

        let mut neighbor_ids = Vec::new();
        {
            let id_mapping = self.id_mapping.read().unwrap();
            for neighbor in &neighbors {
                if let Some(&id) = id_mapping.get(&neighbor.d_id) {
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

            if let Some(cat) = category {
                if memory.category.as_ref() != Some(&cat.to_string()) {
                    continue;
                }
            }

            let score = 1.0 - distance;

            results.push(SearchResult { memory, score });

            if results.len() >= limit {
                break;
            }
        }

        Ok(results)
    }

    pub async fn categorize(&self, content: &str) -> Result<(String, f32)> {
        let embedding = self.embedding_service.generate_embedding(content).await?;

        // Get all memories with categories using the lightweight MemoryRow struct
        // TODO: Use query_as! macro once sqlx-data.json is prepared
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
                let mem_embedding: Vec<f32> = row
                    .embedding_blob
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();
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
