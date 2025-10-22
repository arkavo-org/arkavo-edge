use crate::auth::SessionAuth;
use crate::error::{A2aError, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sqlx::{Row, sqlite::SqlitePool};
use std::path::Path;
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSession {
    pub session_id: String,
    pub auth: Option<SessionAuth>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub conversation_context: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedMessage {
    pub session_id: String,
    pub message_id: String,
    pub content: String,
    pub role: String, // "user" or "assistant"
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[async_trait]
pub trait SessionPersistence: Send + Sync {
    async fn save_session(&self, session: &PersistedSession) -> Result<()>;
    async fn load_session(&self, session_id: &str) -> Result<Option<PersistedSession>>;
    async fn delete_session(&self, session_id: &str) -> Result<()>;
    async fn save_message(&self, message: &PersistedMessage) -> Result<()>;
    async fn load_messages(&self, session_id: &str) -> Result<Vec<PersistedMessage>>;
    async fn cleanup_old_sessions(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize>;
}

pub struct SqliteSessionPersistence {
    pool: SqlitePool,
}

impl SqliteSessionPersistence {
    pub async fn new(db_path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                A2aError::Internal(format!("Failed to create database directory: {e}"))
            })?;
        }

        // Use SQLite connection string with options for creating the database if it doesn't exist
        let db_url = if db_path.to_string_lossy().contains(":memory:") {
            "sqlite::memory:".to_string()
        } else {
            format!("sqlite://{}?mode=rwc", db_path.display())
        };

        let pool = SqlitePool::connect(&db_url)
            .await
            .map_err(|e| A2aError::Internal(format!("Failed to connect to database: {e}")))?;

        // Create tables if they don't exist
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                auth_data TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                conversation_context TEXT
            )
            "#,
        )
        .execute(&pool)
        .await
        .map_err(|e| A2aError::Internal(format!("Failed to create sessions table: {e}")))?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                content TEXT NOT NULL,
                role TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                FOREIGN KEY (session_id) REFERENCES sessions(session_id) ON DELETE CASCADE
            )
            "#,
        )
        .execute(&pool)
        .await
        .map_err(|e| A2aError::Internal(format!("Failed to create messages table: {e}")))?;

        // Create index for session_id in messages table
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_messages_session_id ON messages(session_id)
            "#,
        )
        .execute(&pool)
        .await
        .map_err(|e| A2aError::Internal(format!("Failed to create index: {e}")))?;

        info!(
            "SQLite session persistence initialized at: {}",
            db_path.display()
        );

        Ok(Self { pool })
    }
}

#[async_trait]
impl SessionPersistence for SqliteSessionPersistence {
    async fn save_session(&self, session: &PersistedSession) -> Result<()> {
        let auth_json = session
            .auth
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| A2aError::Internal(format!("Failed to serialize auth: {e}")))?;

        let context_json = serde_json::to_string(&session.conversation_context)
            .map_err(|e| A2aError::Internal(format!("Failed to serialize context: {e}")))?;

        sqlx::query(
            r#"
            INSERT INTO sessions (session_id, auth_data, created_at, updated_at, conversation_context)
            VALUES (?, ?, ?, ?, ?)
            ON CONFLICT(session_id) DO UPDATE SET
                auth_data = excluded.auth_data,
                updated_at = excluded.updated_at,
                conversation_context = excluded.conversation_context
            "#,
        )
        .bind(&session.session_id)
        .bind(auth_json)
        .bind(session.created_at.to_rfc3339())
        .bind(session.updated_at.to_rfc3339())
        .bind(context_json)
        .execute(&self.pool)
        .await
        .map_err(|e| A2aError::Internal(format!("Failed to save session: {e}")))?;

        Ok(())
    }

    async fn load_session(&self, session_id: &str) -> Result<Option<PersistedSession>> {
        let row = sqlx::query(
            r#"
            SELECT session_id, auth_data, created_at, updated_at, conversation_context
            FROM sessions
            WHERE session_id = ?
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| A2aError::Internal(format!("Failed to load session: {e}")))?;

        if let Some(row) = row {
            let auth_data: Option<String> = row.try_get("auth_data").ok();
            let auth = auth_data.and_then(|json| serde_json::from_str::<SessionAuth>(&json).ok());

            let context_json: String = row
                .try_get("conversation_context")
                .unwrap_or_else(|_| "[]".to_string());
            let conversation_context =
                serde_json::from_str(&context_json).unwrap_or_else(|_| Vec::new());

            let created_at = chrono::DateTime::parse_from_rfc3339(
                &row.try_get::<String, _>("created_at").unwrap_or_default(),
            )
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

            let updated_at = chrono::DateTime::parse_from_rfc3339(
                &row.try_get::<String, _>("updated_at").unwrap_or_default(),
            )
            .ok()
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

            Ok(Some(PersistedSession {
                session_id: session_id.to_string(),
                auth,
                created_at,
                updated_at,
                conversation_context,
            }))
        } else {
            Ok(None)
        }
    }

    async fn delete_session(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE session_id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await
            .map_err(|e| A2aError::Internal(format!("Failed to delete session: {e}")))?;

        Ok(())
    }

    async fn save_message(&self, message: &PersistedMessage) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO messages (session_id, message_id, content, role, timestamp)
            VALUES (?, ?, ?, ?, ?)
            "#,
        )
        .bind(&message.session_id)
        .bind(&message.message_id)
        .bind(&message.content)
        .bind(&message.role)
        .bind(message.timestamp.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| A2aError::Internal(format!("Failed to save message: {e}")))?;

        Ok(())
    }

    async fn load_messages(&self, session_id: &str) -> Result<Vec<PersistedMessage>> {
        let rows = sqlx::query(
            r#"
            SELECT message_id, content, role, timestamp
            FROM messages
            WHERE session_id = ?
            ORDER BY timestamp ASC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| A2aError::Internal(format!("Failed to load messages: {e}")))?;

        let messages = rows
            .into_iter()
            .filter_map(|row| {
                let message_id: String = row.try_get("message_id").ok()?;
                let content: String = row.try_get("content").ok()?;
                let role: String = row.try_get("role").ok()?;
                let timestamp_str: String = row.try_get("timestamp").ok()?;

                let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
                    .ok()?
                    .with_timezone(&chrono::Utc);

                Some(PersistedMessage {
                    session_id: session_id.to_string(),
                    message_id,
                    content,
                    role,
                    timestamp,
                })
            })
            .collect();

        Ok(messages)
    }

    async fn cleanup_old_sessions(
        &self,
        older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize> {
        let result = sqlx::query("DELETE FROM sessions WHERE updated_at < ?")
            .bind(older_than.to_rfc3339())
            .execute(&self.pool)
            .await
            .map_err(|e| A2aError::Internal(format!("Failed to cleanup sessions: {e}")))?;

        let deleted = result.rows_affected() as usize;
        if deleted > 0 {
            info!("Cleaned up {} old sessions", deleted);
        }

        Ok(deleted)
    }
}

pub struct NoOpSessionPersistence;

#[async_trait]
impl SessionPersistence for NoOpSessionPersistence {
    async fn save_session(&self, _session: &PersistedSession) -> Result<()> {
        Ok(())
    }

    async fn load_session(&self, _session_id: &str) -> Result<Option<PersistedSession>> {
        Ok(None)
    }

    async fn delete_session(&self, _session_id: &str) -> Result<()> {
        Ok(())
    }

    async fn save_message(&self, _message: &PersistedMessage) -> Result<()> {
        Ok(())
    }

    async fn load_messages(&self, _session_id: &str) -> Result<Vec<PersistedMessage>> {
        Ok(Vec::new())
    }

    async fn cleanup_old_sessions(
        &self,
        _older_than: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize> {
        Ok(0)
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_sqlite_persistence() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("sessions").join("test.db");

        let persistence = SqliteSessionPersistence::new(&db_path).await.unwrap();

        // Create a test session
        let session = PersistedSession {
            session_id: "test-session".to_string(),
            auth: Some(SessionAuth {
                sub: "user123".to_string(),
                scopes: vec!["read".to_string()],
                exp: None,
                metadata: None,
            }),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            conversation_context: vec!["Hello".to_string(), "World".to_string()],
        };

        // Save session
        persistence.save_session(&session).await.unwrap();

        // Load session
        let loaded = persistence.load_session("test-session").await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.session_id, "test-session");
        assert!(loaded.auth.is_some());
        assert_eq!(loaded.conversation_context.len(), 2);

        // Save a message
        let message = PersistedMessage {
            session_id: "test-session".to_string(),
            message_id: "msg1".to_string(),
            content: "Hello, world!".to_string(),
            role: "user".to_string(),
            timestamp: chrono::Utc::now(),
        };
        persistence.save_message(&message).await.unwrap();

        // Load messages
        let messages = persistence.load_messages("test-session").await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Hello, world!");

        // Delete session
        persistence.delete_session("test-session").await.unwrap();
        let loaded = persistence.load_session("test-session").await.unwrap();
        assert!(loaded.is_none());
    }
}
