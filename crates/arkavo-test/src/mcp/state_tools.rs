use super::server::{Tool, ToolSchema};
use crate::state_store::StateStore;
use crate::{Result, TestError};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub struct QueryStateKit {
    schema: ToolSchema,
    state_store: Arc<StateStore>,
}

impl QueryStateKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "query_state".to_string(),
                description: "Query application state".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "entity": {
                            "type": "string",
                            "description": "Entity to query"
                        },
                        "filter": {
                            "type": "object",
                            "description": "Optional filter criteria"
                        }
                    },
                    "required": ["entity"]
                }),
            },
            state_store: Arc::new(StateStore::new()),
        }
    }

    pub fn with_state_store(state_store: Arc<StateStore>) -> Self {
        Self {
            schema: Self::new().schema,
            state_store,
        }
    }
}

impl Default for QueryStateKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for QueryStateKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let entity = params
            .get("entity")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing entity parameter".to_string()))?;

        let filter = params.get("filter").cloned();

        // Query from state store
        let result = if entity == "*" {
            // Query all entities with optional filter
            self.state_store.query(filter.as_ref())?
        } else {
            // Query specific entity
            let state = self.state_store.get(entity)?;
            let mut results = HashMap::new();
            if let Some(s) = state {
                results.insert(entity.to_string(), s);
            }
            results
        };

        Ok(serde_json::json!({
            "state": result,
            "count": result.len(),
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct MutateStateKit {
    schema: ToolSchema,
    state_store: Arc<StateStore>,
}

impl MutateStateKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "mutate_state".to_string(),
                description: "Mutate application state".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "entity": {
                            "type": "string",
                            "description": "Entity to mutate"
                        },
                        "action": {
                            "type": "string",
                            "description": "Action to perform"
                        },
                        "data": {
                            "type": "object",
                            "description": "Data for the mutation"
                        }
                    },
                    "required": ["entity", "action"]
                }),
            },
            state_store: Arc::new(StateStore::new()),
        }
    }

    pub fn with_state_store(state_store: Arc<StateStore>) -> Self {
        Self {
            schema: Self::new().schema,
            state_store,
        }
    }
}

impl Default for MutateStateKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MutateStateKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let entity = params
            .get("entity")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing entity parameter".to_string()))?;

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        let data = params.get("data").cloned();

        // Handle different actions
        let result = match action {
            "set" | "create" => {
                // Set or create entity with provided data
                let value = data.unwrap_or(serde_json::json!({}));
                self.state_store.set(entity, value.clone())?;
                value
            }
            "update" => {
                // Update existing entity
                self.state_store
                    .update(entity, action, data, |current, _, update_data| {
                        match (current, update_data) {
                            (Some(current_val), Some(update_val)) => {
                                // Merge update data into current
                                if let (Some(current_obj), Some(update_obj)) =
                                    (current_val.as_object(), update_val.as_object())
                                {
                                    let mut merged = current_obj.clone();
                                    for (k, v) in update_obj {
                                        merged.insert(k.clone(), v.clone());
                                    }
                                    Ok(serde_json::json!(merged))
                                } else {
                                    Ok(update_val.clone())
                                }
                            }
                            (None, Some(update_val)) => Ok(update_val.clone()),
                            (Some(current_val), None) => Ok(current_val.clone()),
                            (None, None) => Ok(serde_json::json!({})),
                        }
                    })?
            }
            "delete" => {
                // Delete entity
                let existed = self.state_store.delete(entity)?;
                serde_json::json!({ "deleted": existed })
            }
            _ => {
                // Custom action - just store the action and data
                self.state_store.update(
                    entity,
                    action,
                    data,
                    |current, action_name, action_data| {
                        let mut result = current.cloned().unwrap_or(serde_json::json!({}));
                        if let Some(obj) = result.as_object_mut() {
                            obj.insert("last_action".to_string(), serde_json::json!(action_name));
                            if let Some(data) = action_data {
                                obj.insert("last_action_data".to_string(), data.clone());
                            }
                            obj.insert(
                                "last_action_time".to_string(),
                                serde_json::json!(chrono::Utc::now().to_rfc3339()),
                            );
                        }
                        Ok(result)
                    },
                )?
            }
        };

        Ok(serde_json::json!({
            "success": true,
            "entity": entity,
            "action": action,
            "result": result,
            "timestamp": chrono::Utc::now().to_rfc3339()
        }))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

pub struct SnapshotKit {
    schema: ToolSchema,
    state_store: Arc<StateStore>,
}

impl SnapshotKit {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "snapshot".to_string(),
                description: "Create or restore state snapshots".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["create", "restore", "list"],
                            "description": "Snapshot action"
                        },
                        "name": {
                            "type": "string",
                            "description": "Snapshot name"
                        }
                    },
                    "required": ["action"]
                }),
            },
            state_store: Arc::new(StateStore::new()),
        }
    }

    pub fn with_state_store(state_store: Arc<StateStore>) -> Self {
        Self {
            schema: Self::new().schema,
            state_store,
        }
    }
}

impl Default for SnapshotKit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SnapshotKit {
    async fn execute(&self, params: Value) -> Result<Value> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| TestError::Mcp("Missing action parameter".to_string()))?;

        match action {
            "create" => {
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unnamed");

                self.state_store.create_snapshot(name)?;

                Ok(serde_json::json!({
                    "success": true,
                    "snapshot_id": name,
                    "snapshot_name": name,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }))
            }
            "restore" => {
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| TestError::Mcp("Missing snapshot name".to_string()))?;

                self.state_store.restore_snapshot(name)?;

                Ok(serde_json::json!({
                    "success": true,
                    "snapshot_name": name,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }))
            }
            "list" => {
                let snapshots = self.state_store.list_snapshots()?;

                Ok(serde_json::json!({
                    "snapshots": snapshots,
                    "count": snapshots.len(),
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }))
            }
            _ => Err(TestError::Mcp(format!("Invalid action: {}", action))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}