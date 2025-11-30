use crate::{LlmInfo, Router};
use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::server::Tool;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

/// Tool for listing available LLM models.
pub struct ListModelsTool {
    router: Arc<Router>,
    schema: ToolSchema,
}

impl ListModelsTool {
    pub fn new(router: Arc<Router>) -> Self {
        Self {
            router,
            schema: ToolSchema {
                name: "list_models".to_string(),
                aliases: None,
                description: "List all available LLM models with their provider, model name, and availability status. Use this to discover what models can be used for tasks.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for ListModelsTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, _args: Value) -> arkavo_mcp_tools::Result<Value> {
        let models = self.router.get_available_llms();

        if models.is_empty() {
            return Ok(json!({
                "error": "No models available. Check API keys and configuration.",
                "models": []
            }));
        }

        let result = ModelsResponse { models };
        let json = serde_json::to_value(result).map_err(arkavo_mcp_tools::ToolError::Json)?;

        Ok(json)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ModelsResponse {
    models: Vec<LlmInfo>,
}

/// Register router tools with the tool registry
pub fn register_tools(registry: &mut arkavo_mcp_tools::ToolRegistry, router: Arc<Router>) {
    registry.register("list_models", Box::new(ListModelsTool::new(router)));
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_list_models_schema() {
        // Router::new() requires llama-cpp or llama-cpp feature
        if let Ok(router) = Router::new().await {
            let router = Arc::new(router);
            let tool = ListModelsTool::new(router);

            let schema = tool.schema();
            assert_eq!(schema.name, "list_models");
            assert!(schema.description.contains("LLM models"));
        }
    }

    #[tokio::test]
    async fn test_list_models_execute() {
        // Router::new() requires llama-cpp or llama-cpp feature
        if let Ok(router) = Router::new().await {
            let router = Arc::new(router);
            let tool = ListModelsTool::new(router);

            let result = tool.execute(json!({})).await;
            assert!(result.is_ok());

            let response = result.unwrap();
            assert!(response.is_object());
            assert!(response.get("models").is_some());
        }
    }

    #[tokio::test]
    async fn test_list_models_includes_local() {
        // Router::new() requires llama-cpp or llama-cpp feature
        if let Ok(router) = Router::new().await {
            let router = Arc::new(router);
            let tool = ListModelsTool::new(router);

            let result = tool.execute(json!({})).await.unwrap();
            let models = result["models"].as_array().unwrap();

            let has_local = models
                .iter()
                .any(|m| m["name"].as_str().unwrap().contains("Local"));

            assert!(has_local, "Should always include local model");
        }
    }
}
