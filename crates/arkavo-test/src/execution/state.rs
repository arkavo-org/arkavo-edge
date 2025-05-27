use crate::{Result, TestError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub id: String,
    pub name: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub data: Vec<u8>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug)]
pub struct StateManager {
    snapshots: Arc<RwLock<HashMap<String, StateSnapshot>>>,
    current_state: Arc<RwLock<Vec<u8>>>,
}

impl StateManager {
    pub fn new() -> Result<Self> {
        Ok(Self {
            snapshots: Arc::new(RwLock::new(HashMap::new())),
            current_state: Arc::new(RwLock::new(Vec::new())),
        })
    }

    pub fn create_snapshot(&self, name: &str) -> Result<String> {
        let state = self
            .current_state
            .read()
            .map_err(|e| TestError::Execution(format!("Failed to read current state: {}", e)))?;

        let snapshot = StateSnapshot {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            timestamp: chrono::Utc::now(),
            data: state.clone(),
            metadata: HashMap::new(),
        };

        let id = snapshot.id.clone();

        self.snapshots
            .write()
            .map_err(|e| TestError::Execution(format!("Failed to write snapshot: {}", e)))?
            .insert(id.clone(), snapshot);

        Ok(id)
    }

    pub fn restore_snapshot(&self, id: &str) -> Result<()> {
        let snapshots = self
            .snapshots
            .read()
            .map_err(|e| TestError::Execution(format!("Failed to read snapshots: {}", e)))?;

        let snapshot = snapshots
            .get(id)
            .ok_or_else(|| TestError::Execution(format!("Snapshot not found: {}", id)))?;

        let mut state = self
            .current_state
            .write()
            .map_err(|e| TestError::Execution(format!("Failed to write current state: {}", e)))?;

        *state = snapshot.data.clone();

        Ok(())
    }

    pub fn list_snapshots(&self) -> Result<Vec<StateSnapshot>> {
        let snapshots = self
            .snapshots
            .read()
            .map_err(|e| TestError::Execution(format!("Failed to read snapshots: {}", e)))?;

        let mut list: Vec<StateSnapshot> = snapshots.values().cloned().collect();
        list.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        Ok(list)
    }

    pub fn delete_snapshot(&self, id: &str) -> Result<()> {
        self.snapshots
            .write()
            .map_err(|e| TestError::Execution(format!("Failed to write snapshots: {}", e)))?
            .remove(id)
            .ok_or_else(|| TestError::Execution(format!("Snapshot not found: {}", id)))?;

        Ok(())
    }

    pub fn branch_snapshot(&self, from_id: &str, new_name: &str) -> Result<String> {
        let snapshots = self
            .snapshots
            .read()
            .map_err(|e| TestError::Execution(format!("Failed to read snapshots: {}", e)))?;

        let parent = snapshots.get(from_id).ok_or_else(|| {
            TestError::Execution(format!("Parent snapshot not found: {}", from_id))
        })?;

        let new_snapshot = StateSnapshot {
            id: Uuid::new_v4().to_string(),
            name: new_name.to_string(),
            timestamp: chrono::Utc::now(),
            data: parent.data.clone(),
            metadata: {
                let mut meta = parent.metadata.clone();
                meta.insert("parent_id".to_string(), from_id.to_string());
                meta
            },
        };

        let id = new_snapshot.id.clone();

        drop(snapshots);

        self.snapshots
            .write()
            .map_err(|e| TestError::Execution(format!("Failed to write snapshots: {}", e)))?
            .insert(id.clone(), new_snapshot);

        Ok(id)
    }

    pub fn get_current_state(&self) -> Result<Vec<u8>> {
        let state = self
            .current_state
            .read()
            .map_err(|e| TestError::Execution(format!("Failed to read current state: {}", e)))?;

        Ok(state.clone())
    }

    pub fn set_current_state(&self, data: Vec<u8>) -> Result<()> {
        let mut state = self
            .current_state
            .write()
            .map_err(|e| TestError::Execution(format!("Failed to write current state: {}", e)))?;

        *state = data;

        Ok(())
    }
}
