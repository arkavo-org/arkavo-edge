use crate::error::{GeminiError, Result};
use crate::types::FunctionCall;
use dashmap::DashMap;
use serde_json::Value;
use std::collections::HashSet;
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
    in_flight_ids: Arc<DashMap<String, ()>>,
}

impl ToolDispatcher {
    pub fn new(max_concurrency: usize) -> Self {
        Self {
            tools: DashMap::new(),
            semaphore: Arc::new(Semaphore::new(max_concurrency)),
            processed_ids: Arc::new(DashMap::new()),
            in_flight_ids: Arc::new(DashMap::new()),
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
    /// Tool handlers may still panic internally; panic join errors are converted
    /// into per-call `ToolExecutionError` results.
    pub async fn dispatch(&self, calls: Vec<FunctionCall>) -> Vec<(String, Result<Value>)> {
        let mut results = Vec::new();
        let mut tasks = Vec::new();
        let mut batch_seen_ids = HashSet::new();

        for call in calls {
            if !batch_seen_ids.insert(call.id.clone()) {
                debug!("Skipping duplicate tool call in batch: {}", call.id);
                continue;
            }

            if self.is_duplicate(&call.id) {
                debug!("Skipping duplicate tool call: {}", call.id);
                continue;
            }

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

            self.mark_in_flight(&call.id);
            let semaphore = self.semaphore.clone();
            let task = tokio::spawn(async move {
                let _permit = semaphore.acquire_owned().await.map_err(|e| {
                    GeminiError::ToolExecutionError(format!("Semaphore closed: {e}"))
                })?;
                Self::execute_tool(&tool, call.args)
            });

            tasks.push((call.id, task));
        }

        for (call_id, task) in tasks {
            match task.await {
                Ok(result) => {
                    if result.is_ok() {
                        self.mark_processed(&call_id);
                    }
                    results.push((call_id.clone(), result));
                }
                Err(e) => results.push((
                    call_id.clone(),
                    Err(GeminiError::ToolExecutionError(format!(
                        "Tool call task failed: {e}"
                    ))),
                )),
            }
            self.clear_in_flight(&call_id);
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
        self.processed_ids.contains_key(id) || self.in_flight_ids.contains_key(id)
    }

    fn mark_processed(&self, id: &str) {
        self.processed_ids.insert(id.to_string(), ());
    }

    fn mark_in_flight(&self, id: &str) {
        self.in_flight_ids.insert(id.to_string(), ());
    }

    fn clear_in_flight(&self, id: &str) {
        self.in_flight_ids.remove(id);
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
    use arkavo_test_macros::spec;
    use uuid::Uuid;

    #[spec("GEM-007")]
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

    #[spec("GEM-007")]
    #[tokio::test]
    async fn test_idempotency() {
        let dispatcher = ToolDispatcher::new(4);
        let id = Uuid::new_v4().to_string();
        let mut registry = ToolRegistry::new();

        registry.register(
            "test",
            "idempotency test tool",
            serde_json::json!({"type": "object"}),
            |_args| Ok(serde_json::json!({"ok": true})),
        );
        registry.build(&dispatcher);

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

    #[spec("GEM-007")]
    #[tokio::test]
    async fn test_failed_call_id_can_be_retried() {
        let dispatcher = ToolDispatcher::new(4);
        let mut registry = ToolRegistry::new();
        let call_id = Uuid::new_v4().to_string();

        registry.register(
            "flaky_tool",
            "Fails once then succeeds",
            serde_json::json!({"type": "object"}),
            {
                let failed_once = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                move |_args| {
                    if !failed_once.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        return Err(GeminiError::ToolExecutionError(
                            "transient failure".to_string(),
                        ));
                    }
                    Ok(serde_json::json!({"ok": true}))
                }
            },
        );
        registry.build(&dispatcher);

        let first = dispatcher
            .dispatch(vec![FunctionCall {
                id: call_id.clone(),
                name: "flaky_tool".to_string(),
                args: serde_json::json!({}),
            }])
            .await;
        assert_eq!(first.len(), 1);
        assert!(first[0].1.is_err());

        let second = dispatcher
            .dispatch(vec![FunctionCall {
                id: call_id.clone(),
                name: "flaky_tool".to_string(),
                args: serde_json::json!({}),
            }])
            .await;
        assert_eq!(second.len(), 1);
        assert!(second[0].1.is_ok());
    }

    #[spec("GEM-007")]
    #[tokio::test]
    async fn test_successful_call_id_is_not_reprocessed() {
        let dispatcher = ToolDispatcher::new(4);
        let mut registry = ToolRegistry::new();
        let call_id = Uuid::new_v4().to_string();

        registry.register(
            "ok_tool",
            "Always succeeds",
            serde_json::json!({"type": "object"}),
            |_args| Ok(serde_json::json!({"ok": true})),
        );
        registry.build(&dispatcher);

        let first = dispatcher
            .dispatch(vec![FunctionCall {
                id: call_id.clone(),
                name: "ok_tool".to_string(),
                args: serde_json::json!({}),
            }])
            .await;
        assert_eq!(first.len(), 1);
        assert!(first[0].1.is_ok());

        let second = dispatcher
            .dispatch(vec![FunctionCall {
                id: call_id.clone(),
                name: "ok_tool".to_string(),
                args: serde_json::json!({}),
            }])
            .await;
        assert_eq!(second.len(), 0);
    }

    #[spec("GEM-007")]
    #[spec("GEM-008")]
    #[tokio::test]
    async fn test_dispatch_returns_error_when_tool_task_panics() {
        let dispatcher = ToolDispatcher::new(4);
        let mut registry = ToolRegistry::new();
        let call_id = Uuid::new_v4().to_string();

        registry.register(
            "panic_tool",
            "Panics to test task error handling",
            serde_json::json!({"type": "object"}),
            |_args| {
                panic!("boom");
            },
        );
        registry.build(&dispatcher);

        let calls = vec![FunctionCall {
            id: call_id.clone(),
            name: "panic_tool".to_string(),
            args: serde_json::json!({}),
        }];

        let results = dispatcher.dispatch(calls).await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, call_id);
        match &results[0].1 {
            Err(GeminiError::ToolExecutionError(message)) => {
                assert!(message.contains("task failed"));
            }
            other => panic!("expected ToolExecutionError, got {other:?}"),
        }
    }
}
