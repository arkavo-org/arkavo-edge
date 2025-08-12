use crate::{Result, ToolError};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// A simple in-memory state store for MCP tools
#[derive(Debug, Clone)]
pub struct StateStore {
    data: Arc<RwLock<HashMap<String, Value>>>,
    snapshots: Arc<RwLock<HashMap<String, HashMap<String, Value>>>>,
}

impl StateStore {
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
            snapshots: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn get(&self, entity: &str) -> Result<Option<Value>> {
        let data = self
            .data
            .read()
            .map_err(|e| ToolError::Mcp(format!("Failed to read state: {e}")))?;
        Ok(data.get(entity).cloned())
    }

    pub fn set(&self, entity: &str, value: Value) -> Result<()> {
        let mut data = self
            .data
            .write()
            .map_err(|e| ToolError::Mcp(format!("Failed to write state: {e}")))?;
        data.insert(entity.to_string(), value);
        drop(data); // Explicitly drop to avoid contention
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
        let mut data = self
            .data
            .write()
            .map_err(|e| ToolError::Mcp(format!("Failed to write state: {e}")))?;

        let current = data.get(entity);
        let new_value = updater(current, action, update_data.as_ref())?;
        data.insert(entity.to_string(), new_value.clone());
        drop(data);

        Ok(new_value)
    }

    pub fn delete(&self, entity: &str) -> Result<bool> {
        let mut data = self
            .data
            .write()
            .map_err(|e| ToolError::Mcp(format!("Failed to write state: {e}")))?;
        Ok(data.remove(entity).is_some())
    }

    pub fn query(&self, filter: Option<&Value>) -> Result<HashMap<String, Value>> {
        let data = self
            .data
            .read()
            .map_err(|e| ToolError::Mcp(format!("Failed to read state: {e}")))?;

        // Simple implementation: return all data if no filter, or filtered data
        if let Some(_filter) = filter {
            // For now, just return all data - could implement filtering logic later
            Ok(data.clone())
        } else {
            Ok(data.clone())
        }
    }

    pub fn create_snapshot(&self, name: &str) -> Result<()> {
        let data = self
            .data
            .read()
            .map_err(|e| ToolError::Mcp(format!("Failed to read state: {e}")))?;
        let snapshot_data = data.clone();
        drop(data);

        let mut snapshots = self
            .snapshots
            .write()
            .map_err(|e| ToolError::Mcp(format!("Failed to write snapshots: {e}")))?;

        snapshots.insert(name.to_string(), snapshot_data);
        drop(snapshots);
        Ok(())
    }

    pub fn restore_snapshot(&self, name: &str) -> Result<()> {
        let snapshots = self
            .snapshots
            .read()
            .map_err(|e| ToolError::Mcp(format!("Failed to read snapshots: {e}")))?;

        let snapshot = snapshots
            .get(name)
            .ok_or_else(|| ToolError::Mcp(format!("Snapshot '{name}' not found")))?;
        let snapshot_data = snapshot.clone();
        drop(snapshots);

        let mut data = self
            .data
            .write()
            .map_err(|e| ToolError::Mcp(format!("Failed to write state: {e}")))?;

        *data = snapshot_data;
        drop(data);
        Ok(())
    }

    pub fn list_snapshots(&self) -> Result<Vec<String>> {
        let snapshots = self
            .snapshots
            .read()
            .map_err(|e| ToolError::Mcp(format!("Failed to read snapshots: {e}")))?;
        Ok(snapshots.keys().cloned().collect())
    }
}

impl Default for StateStore {
    fn default() -> Self {
        Self::new()
    }
}
