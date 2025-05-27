use crate::{Result, TestError};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

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
            .map_err(|e| TestError::Mcp(format!("Failed to read state: {}", e)))?;
        Ok(data.get(entity).cloned())
    }

    pub fn set(&self, entity: &str, value: Value) -> Result<()> {
        let mut data = self
            .data
            .write()
            .map_err(|e| TestError::Mcp(format!("Failed to write state: {}", e)))?;
        data.insert(entity.to_string(), value);
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
            .map_err(|e| TestError::Mcp(format!("Failed to write state: {}", e)))?;

        let current = data.get(entity);
        let new_value = updater(current, action, update_data.as_ref())?;
        data.insert(entity.to_string(), new_value.clone());

        Ok(new_value)
    }

    pub fn delete(&self, entity: &str) -> Result<bool> {
        let mut data = self
            .data
            .write()
            .map_err(|e| TestError::Mcp(format!("Failed to write state: {}", e)))?;
        Ok(data.remove(entity).is_some())
    }

    pub fn create_snapshot(&self, name: &str) -> Result<()> {
        let data = self
            .data
            .read()
            .map_err(|e| TestError::Mcp(format!("Failed to read state: {}", e)))?;
        let mut snapshots = self
            .snapshots
            .write()
            .map_err(|e| TestError::Mcp(format!("Failed to write snapshots: {}", e)))?;

        snapshots.insert(name.to_string(), data.clone());
        Ok(())
    }

    pub fn restore_snapshot(&self, name: &str) -> Result<()> {
        let snapshots = self
            .snapshots
            .read()
            .map_err(|e| TestError::Mcp(format!("Failed to read snapshots: {}", e)))?;

        let snapshot = snapshots
            .get(name)
            .ok_or_else(|| TestError::Mcp(format!("Snapshot '{}' not found", name)))?;

        let mut data = self
            .data
            .write()
            .map_err(|e| TestError::Mcp(format!("Failed to write state: {}", e)))?;

        *data = snapshot.clone();
        Ok(())
    }

    pub fn list_snapshots(&self) -> Result<Vec<String>> {
        let snapshots = self
            .snapshots
            .read()
            .map_err(|e| TestError::Mcp(format!("Failed to read snapshots: {}", e)))?;
        Ok(snapshots.keys().cloned().collect())
    }

    pub fn query(&self, filter: Option<&Value>) -> Result<HashMap<String, Value>> {
        let data = self
            .data
            .read()
            .map_err(|e| TestError::Mcp(format!("Failed to read state: {}", e)))?;

        if let Some(filter_value) = filter {
            if let Some(filter_obj) = filter_value.as_object() {
                let mut results = HashMap::new();

                for (key, value) in data.iter() {
                    if Self::matches_filter(value, filter_obj) {
                        results.insert(key.clone(), value.clone());
                    }
                }

                Ok(results)
            } else {
                Ok(data.clone())
            }
        } else {
            Ok(data.clone())
        }
    }

    fn matches_filter(value: &Value, filter: &serde_json::Map<String, Value>) -> bool {
        if let Some(value_obj) = value.as_object() {
            for (key, filter_val) in filter {
                if value_obj.get(key) != Some(filter_val) {
                    return false;
                }
            }
            true
        } else {
            false
        }
    }
}

impl Default for StateStore {
    fn default() -> Self {
        Self::new()
    }
}
