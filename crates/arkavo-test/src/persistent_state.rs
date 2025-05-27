use crate::{Result, TestError};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::state_store::StateStore;

/// Configuration for persistent state storage
#[derive(Debug, Clone)]
pub struct PersistentConfig {
    /// Directory where state files will be stored
    pub storage_dir: PathBuf,
    /// Whether to auto-save on every mutation
    pub auto_save: bool,
    /// File name for the main state file
    pub state_file: String,
    /// Directory name for snapshots
    pub snapshot_dir: String,
}

impl Default for PersistentConfig {
    fn default() -> Self {
        Self {
            storage_dir: PathBuf::from(".arkavo/state"),
            auto_save: true,
            state_file: "state.json".to_string(),
            snapshot_dir: "snapshots".to_string(),
        }
    }
}

/// Persistent state store that saves to disk
#[derive(Debug)]
pub struct PersistentStateStore {
    inner: StateStore,
    config: PersistentConfig,
}

impl PersistentStateStore {
    /// Create a new persistent state store with default config
    pub fn new() -> Result<Self> {
        Self::with_config(PersistentConfig::default())
    }

    /// Create a new persistent state store with custom config
    pub fn with_config(config: PersistentConfig) -> Result<Self> {
        // Create directories if they don't exist
        fs::create_dir_all(&config.storage_dir)
            .map_err(|e| TestError::Io(e))?;
        
        let snapshot_path = config.storage_dir.join(&config.snapshot_dir);
        fs::create_dir_all(&snapshot_path)
            .map_err(|e| TestError::Io(e))?;

        let mut store = Self {
            inner: StateStore::new(),
            config,
        };

        // Load existing state if available
        store.load()?;

        Ok(store)
    }

    /// Load state from disk
    pub fn load(&mut self) -> Result<()> {
        let state_path = self.config.storage_dir.join(&self.config.state_file);
        
        if state_path.exists() {
            let data = fs::read_to_string(&state_path)
                .map_err(|e| TestError::Io(e))?;
            
            let state: HashMap<String, Value> = serde_json::from_str(&data)
                .map_err(|e| TestError::Serialization(e))?;
            
            // Load each entry into the inner store
            for (key, value) in state {
                self.inner.set(&key, value)?;
            }
        }

        // Load snapshots
        let snapshot_dir = self.config.storage_dir.join(&self.config.snapshot_dir);
        if snapshot_dir.exists() {
            for entry in fs::read_dir(&snapshot_dir).map_err(|e| TestError::Io(e))? {
                let entry = entry.map_err(|e| TestError::Io(e))?;
                let path = entry.path();
                
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                        let data = fs::read_to_string(&path)
                            .map_err(|e| TestError::Io(e))?;
                        
                        let snapshot: HashMap<String, Value> = serde_json::from_str(&data)
                            .map_err(|e| TestError::Serialization(e))?;
                        
                        // Create snapshot in memory
                        self.inner.create_snapshot(name)?;
                        
                        // Temporarily swap state to load snapshot data
                        for (key, value) in snapshot {
                            self.inner.set(&key, value)?;
                        }
                        self.inner.create_snapshot(name)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Save current state to disk
    pub fn save(&self) -> Result<()> {
        let state_path = self.config.storage_dir.join(&self.config.state_file);
        
        // Get current state
        let state = self.inner.query(None)?;
        
        // Serialize to JSON
        let json = serde_json::to_string_pretty(&state)
            .map_err(|e| TestError::Serialization(e))?;
        
        // Write to file
        fs::write(&state_path, json)
            .map_err(|e| TestError::Io(e))?;
        
        Ok(())
    }

    /// Save a specific snapshot to disk
    pub fn save_snapshot(&self, name: &str) -> Result<()> {
        // First ensure the snapshot exists in memory
        let snapshots = self.inner.list_snapshots()?;
        if !snapshots.contains(&name.to_string()) {
            return Err(TestError::Validation(format!("Snapshot '{}' not found", name)));
        }

        // Get snapshot data by temporarily restoring it
        let current_state = self.inner.query(None)?;
        self.inner.restore_snapshot(name)?;
        let snapshot_state = self.inner.query(None)?;
        
        // Restore original state
        for (key, value) in current_state {
            self.inner.set(&key, value)?;
        }

        // Save snapshot to disk
        let snapshot_path = self.config.storage_dir
            .join(&self.config.snapshot_dir)
            .join(format!("{}.json", name));
        
        let json = serde_json::to_string_pretty(&snapshot_state)
            .map_err(|e| TestError::Serialization(e))?;
        
        fs::write(&snapshot_path, json)
            .map_err(|e| TestError::Io(e))?;
        
        Ok(())
    }

    // Delegate methods to inner store, with auto-save if enabled

    pub fn get(&self, entity: &str) -> Result<Option<Value>> {
        self.inner.get(entity)
    }

    pub fn set(&self, entity: &str, value: Value) -> Result<()> {
        self.inner.set(entity, value)?;
        if self.config.auto_save {
            self.save()?;
        }
        Ok(())
    }

    pub fn update<F>(
        &self,
        entity: &str,
        action: &str,
        update_data: Option<Value>,
        updater: F,
    ) -> Result<Value>
    where
        F: FnOnce(Option<&Value>, &str, Option<&Value>) -> Result<Value>,
    {
        let result = self.inner.update(entity, action, update_data, updater)?;
        if self.config.auto_save {
            self.save()?;
        }
        Ok(result)
    }

    pub fn delete(&self, entity: &str) -> Result<bool> {
        let deleted = self.inner.delete(entity)?;
        if deleted && self.config.auto_save {
            self.save()?;
        }
        Ok(deleted)
    }

    pub fn create_snapshot(&self, name: &str) -> Result<()> {
        self.inner.create_snapshot(name)?;
        if self.config.auto_save {
            self.save_snapshot(name)?;
        }
        Ok(())
    }

    pub fn restore_snapshot(&self, name: &str) -> Result<()> {
        self.inner.restore_snapshot(name)?;
        if self.config.auto_save {
            self.save()?;
        }
        Ok(())
    }

    pub fn list_snapshots(&self) -> Result<Vec<String>> {
        self.inner.list_snapshots()
    }

    pub fn query(&self, filter: Option<&Value>) -> Result<HashMap<String, Value>> {
        self.inner.query(filter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_persistent_store_save_load() -> Result<()> {
        let temp_dir = TempDir::new().unwrap();
        let config = PersistentConfig {
            storage_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        // Create store and add data
        {
            let store = PersistentStateStore::with_config(config.clone())?;
            store.set("user", serde_json::json!({"name": "Alice", "age": 30}))?;
            store.set("config", serde_json::json!({"theme": "dark"}))?;
        }

        // Create new store and verify data persisted
        {
            let store = PersistentStateStore::with_config(config)?;
            let user = store.get("user")?.unwrap();
            assert_eq!(user["name"], "Alice");
            assert_eq!(user["age"], 30);
            
            let config = store.get("config")?.unwrap();
            assert_eq!(config["theme"], "dark");
        }

        Ok(())
    }

    #[test]
    fn test_persistent_snapshots() -> Result<()> {
        let temp_dir = TempDir::new().unwrap();
        let config = PersistentConfig {
            storage_dir: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        // Create store with snapshots
        {
            let store = PersistentStateStore::with_config(config.clone())?;
            store.set("counter", serde_json::json!({"value": 1}))?;
            store.create_snapshot("v1")?;
            
            store.set("counter", serde_json::json!({"value": 2}))?;
            store.create_snapshot("v2")?;
        }

        // Verify snapshots persist
        {
            let store = PersistentStateStore::with_config(config)?;
            let snapshots = store.list_snapshots()?;
            assert!(snapshots.contains(&"v1".to_string()));
            assert!(snapshots.contains(&"v2".to_string()));
            
            // Verify we can restore
            store.restore_snapshot("v1")?;
            let counter = store.get("counter")?.unwrap();
            assert_eq!(counter["value"], 1);
        }

        Ok(())
    }
}