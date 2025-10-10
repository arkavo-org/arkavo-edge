pub mod diff;
pub mod validator;
pub mod ws_handler;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeUpdate {
    pub id: String,
    pub html: Option<String>,
    pub css: Option<String>,
    pub javascript: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub source: UpdateSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UpdateSource {
    LlmGenerated,
    BrowserEdited,
    CodingAgentRefined,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeVersion {
    pub version_id: String,
    pub update: CodeUpdate,
    pub parent_version: Option<String>,
    pub diff_summary: String,
}

pub struct LiveReloadManager {
    versions: Arc<RwLock<HashMap<String, CodeVersion>>>,
    current_version: Arc<RwLock<Option<String>>>,
    subscribers: Arc<RwLock<Vec<tokio::sync::mpsc::Sender<CodeUpdate>>>>,
}

impl LiveReloadManager {
    pub fn new() -> Self {
        Self {
            versions: Arc::new(RwLock::new(HashMap::new())),
            current_version: Arc::new(RwLock::new(None)),
            subscribers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn apply_update(&self, update: CodeUpdate) -> Result<String> {
        let validator = validator::CodeValidator::new();
        validator.validate(&update)?;

        let version_id = uuid::Uuid::new_v4().to_string();
        let parent_version = self.current_version.read().await.clone();

        let diff_summary = if let Some(parent_id) = &parent_version {
            if let Some(parent) = self.versions.read().await.get(parent_id) {
                diff::calculate_diff_summary(&parent.update, &update)
            } else {
                "Initial version".to_string()
            }
        } else {
            "Initial version".to_string()
        };

        let version = CodeVersion {
            version_id: version_id.clone(),
            update: update.clone(),
            parent_version,
            diff_summary,
        };

        self.versions
            .write()
            .await
            .insert(version_id.clone(), version);
        *self.current_version.write().await = Some(version_id.clone());

        self.broadcast_update(update).await;

        Ok(version_id)
    }

    async fn broadcast_update(&self, update: CodeUpdate) {
        let subscribers = self.subscribers.read().await;
        for tx in subscribers.iter() {
            let _ = tx.send(update.clone()).await;
        }
    }

    pub async fn subscribe(&self) -> tokio::sync::mpsc::Receiver<CodeUpdate> {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        self.subscribers.write().await.push(tx);
        rx
    }

    pub async fn get_current_version(&self) -> Option<CodeVersion> {
        let current_id = self.current_version.read().await.clone()?;
        self.versions.read().await.get(&current_id).cloned()
    }

    pub async fn get_version_history(&self, limit: usize) -> Vec<CodeVersion> {
        let versions = self.versions.read().await;
        let mut history: Vec<_> = versions.values().cloned().collect();
        history.sort_by(|a, b| b.update.timestamp.cmp(&a.update.timestamp));
        history.into_iter().take(limit).collect()
    }

    pub async fn rollback(&self, version_id: &str) -> Result<()> {
        let versions = self.versions.read().await;
        if versions.contains_key(version_id) {
            *self.current_version.write().await = Some(version_id.to_string());
            if let Some(version) = versions.get(version_id) {
                self.broadcast_update(version.update.clone()).await;
            }
            Ok(())
        } else {
            Err(anyhow::anyhow!("Version not found"))
        }
    }
}

impl Default for LiveReloadManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_update_application() {
        let manager = LiveReloadManager::new();
        let update = CodeUpdate {
            id: uuid::Uuid::new_v4().to_string(),
            html: Some("<div>Test</div>".to_string()),
            css: Some(".test { color: red; }".to_string()),
            javascript: Some("console.log('test');".to_string()),
            timestamp: chrono::Utc::now(),
            source: UpdateSource::LlmGenerated,
        };

        let result = manager.apply_update(update).await;
        assert!(result.is_ok());

        let current = manager.get_current_version().await;
        assert!(current.is_some());
    }

    #[tokio::test]
    async fn test_version_history() {
        let manager = LiveReloadManager::new();

        for i in 0..3 {
            let update = CodeUpdate {
                id: uuid::Uuid::new_v4().to_string(),
                html: Some(format!("<div>Test {i}</div>")),
                css: None,
                javascript: None,
                timestamp: chrono::Utc::now(),
                source: UpdateSource::LlmGenerated,
            };
            manager.apply_update(update).await.unwrap();
        }

        let history = manager.get_version_history(10).await;
        assert_eq!(history.len(), 3);
    }
}
