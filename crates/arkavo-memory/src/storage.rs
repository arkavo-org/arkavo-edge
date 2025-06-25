use crate::embeddings::EmbeddingService;
use crate::error::{MemoryError, Result};
use crate::models::{Memory, SearchResult};
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
    embedding: String, // JSON-encoded Vec<f32>
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
    #[allow(dead_code)]
    config: HnswConfig,
}

impl MemoryStorage {
    pub async fn new() -> Result<Self> {
        Self::with_config(HnswConfig::default()).await
    }

    pub fn get_data_directory() -> Result<PathBuf> {
        let data_dir = PathBuf::from(".arkavo").join("memory_server");

        std::fs::create_dir_all(&data_dir)
            .map_err(|e| MemoryError::Storage(format!("Failed to create data directory: {}", e)))?;

        Ok(data_dir)
    }

    pub async fn with_config(config: HnswConfig) -> Result<Self> {
        let data_dir = Self::get_data_directory()?;
        let db_path = data_dir.join("memories.db");

        #[cfg(debug_assertions)]
        {
            eprintln!("Memory storage: Creating database at {:?}", db_path);
            eprintln!("Current directory: {:?}", std::env::current_dir());
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

        let mut storage = Self {
            pool,
            embeddings: Arc::new(RwLock::new(HashMap::new())),
            index: Arc::new(RwLock::new(hnsw)),
            id_mapping: Arc::new(RwLock::new(HashMap::new())),
            embedding_service: EmbeddingService::new(),
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
                embedding TEXT NOT NULL,
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
        let rows = sqlx::query("SELECT id, embedding FROM memories")
            .fetch_all(&self.pool)
            .await?;

        let mut embeddings = self.embeddings.write().unwrap();
        let index = self.index.write().unwrap();
        let mut id_mapping = self.id_mapping.write().unwrap();

        for (idx, row) in rows.iter().enumerate() {
            let id_str: String = row.get("id");
            let embedding_str: String = row.get("embedding");

            let id = Uuid::parse_str(&id_str)
                .map_err(|e| MemoryError::Storage(format!("Invalid UUID: {}", e)))?;
            let embedding: Vec<f32> = serde_json::from_str(&embedding_str)
                .map_err(|e| MemoryError::Storage(format!("Invalid embedding data: {}", e)))?;

            embeddings.insert(id, embedding.clone());
            id_mapping.insert(idx, id);

            let mut point_data = Vec::with_capacity(embedding.len());
            point_data.extend_from_slice(&embedding);
            index.insert((&point_data, idx));
        }

        Ok(())
    }

    fn get_next_index(&self) -> usize {
        let id_mapping = self.id_mapping.read().unwrap();
        id_mapping.len()
    }

    pub async fn store(&self, memory: Memory) -> Result<()> {
        let id_str = memory.id.to_string();
        let embedding_json = serde_json::to_string(&memory.embedding)?;
        let metadata_json = memory
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;

        sqlx::query(
            r#"
            INSERT INTO memories (id, content, metadata, category, embedding, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#
        )
        .bind(&id_str)
        .bind(&memory.content)
        .bind(&metadata_json)
        .bind(&memory.category)
        .bind(&embedding_json)
        .bind(memory.created_at.to_rfc3339())
        .bind(memory.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

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

        Ok(())
    }

    pub async fn get(&self, id: Uuid) -> Result<Memory> {
        let id_str = id.to_string();
        let row = sqlx::query("SELECT * FROM memories WHERE id = ?")
            .bind(&id_str)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(MemoryError::NotFound)?;

        let embedding_str: String = row.get("embedding");
        let embedding: Vec<f32> = serde_json::from_str(&embedding_str)?;

        let metadata_str: Option<String> = row.get("metadata");
        let metadata = metadata_str
            .as_ref()
            .map(|m| serde_json::from_str(m))
            .transpose()?;

        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");

        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| MemoryError::Storage(format!("Invalid created_at timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| MemoryError::Storage(format!("Invalid updated_at timestamp: {}", e)))?
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
        let rows: Vec<MemoryRow> = sqlx::query_as(
            "SELECT id, category, embedding
             FROM memories
             WHERE category IS NOT NULL AND embedding IS NOT NULL",
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
                // Parse the embedding from JSON
                let mem_embedding: Vec<f32> = serde_json::from_str(&row.embedding)?;
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
}
