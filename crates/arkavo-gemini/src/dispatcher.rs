use crate::error::{GeminiError, Result};
use crate::types::FunctionCall;
use dashmap::DashMap;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{debug, warn};

pub type ToolHandler = Box<dyn Fn(Value) -> Result<Value> + Send + Sync>;

pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub handler: ToolHandler,
}

pub struct ToolDispatcher {
    tools: DashMap<String, Arc<ToolDefinition>>,
    semaphore: Arc<Semaphore>,
    processed_ids: Arc<DashMap<String, ()>>,
}

impl ToolDispatcher {
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            tools: DashMap::new(),
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            processed_ids: Arc::new(DashMap::new()),
        }
    }

    pub fn register_tool(&self, definition: ToolDefinition) {
        let name = definition.name.clone();
        self.tools.insert(name, Arc::new(definition));
    }

    pub fn list_tools(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "name": entry.value().name,
                    "description": entry.value().description,
                    "parameters": entry.value().schema,
                })
            })
            .collect()
    }

    /// Dispatches tool calls concurrently up to the configured limit.
    ///
    /// # Panics
    ///
    /// Panics if the semaphore is closed unexpectedly.
    pub async fn dispatch(&self, calls: Vec<FunctionCall>) -> Vec<(String, Result<Value>)> {
        let mut results = Vec::new();
        let mut tasks = Vec::new();

        for call in calls {
            if self.is_duplicate(&call.id) {
                debug!("Skipping duplicate tool call: {}", call.id);
                continue;
            }

            self.mark_processed(&call.id);

            let tool = match self.tools.get(&call.name) {
                Some(t) => t.clone(),
                None => {
                    warn!("Tool not found: {}", call.name);
                    results.push((
                        call.id,
                        Err(GeminiError::ToolExecutionError(format!(
                            "Tool '{}' not found",
                            call.name
                        ))),
                    ));
                    continue;
                }
            };

            let semaphore = self.semaphore.clone();
            let task = tokio::spawn(async move {
                let _permit = semaphore.acquire().await.expect("Semaphore closed");
                let result = Self::execute_tool(&tool, call.args);
                (call.id, result)
            });

            tasks.push(task);
        }

        for task in tasks {
            if let Ok(result) = task.await {
                results.push(result);
            }
        }

        results
    }

    #[allow(clippy::result_large_err)]
    fn execute_tool(tool: &ToolDefinition, args: Value) -> Result<Value> {
        if let Err(e) = Self::validate_args(&tool.schema, &args) {
            return Err(GeminiError::SchemaValidationError(e));
        }

        (tool.handler)(args)
    }

    fn validate_args(schema: &Value, args: &Value) -> std::result::Result<(), String> {
        if !args.is_object() {
            return Err("Arguments must be an object".to_string());
        }

        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            let args_obj = args.as_object().unwrap();
            for field in required {
                if let Some(field_name) = field.as_str()
                    && !args_obj.contains_key(field_name)
                {
                    return Err(format!("Missing required field: {field_name}"));
                }
            }
        }

        Ok(())
    }

    fn is_duplicate(&self, id: &str) -> bool {
        self.processed_ids.contains_key(id)
    }

    fn mark_processed(&self, id: &str) {
        self.processed_ids.insert(id.to_string(), ());
    }

    pub fn clear_processed_ids(&self) {
        self.processed_ids.clear();
    }
}

pub struct ToolRegistry {
    tools: Vec<ToolDefinition>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register<F>(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        schema: Value,
        handler: F,
    ) where
        F: Fn(Value) -> Result<Value> + Send + Sync + 'static,
    {
        self.tools.push(ToolDefinition {
            name: name.into(),
            description: description.into(),
            schema,
            handler: Box::new(handler),
        });
    }

    pub fn build(self, dispatcher: &ToolDispatcher) {
        for tool in self.tools {
            dispatcher.register_tool(tool);
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_tool_registration() {
        let dispatcher = ToolDispatcher::new(4);
        let mut registry = ToolRegistry::new();

        registry.register(
            "test_tool",
            "A test tool",
            serde_json::json!({"type": "object", "required": ["param1"]}),
            |_args| Ok(serde_json::json!({"result": "success"})),
        );

        registry.build(&dispatcher);
        assert_eq!(dispatcher.list_tools().len(), 1);
    }

    #[tokio::test]
    async fn test_idempotency() {
        let dispatcher = ToolDispatcher::new(4);
        let id = Uuid::new_v4().to_string();

        let calls = vec![
            FunctionCall {
                id: id.clone(),
                name: "test".to_string(),
                args: serde_json::json!({}),
            },
            FunctionCall {
                id: id.clone(),
                name: "test".to_string(),
                args: serde_json::json!({}),
            },
        ];

        let results = dispatcher.dispatch(calls).await;
        assert_eq!(results.len(), 1);
    }
}
