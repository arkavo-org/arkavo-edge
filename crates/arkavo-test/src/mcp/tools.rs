use crate::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: ParameterSchema,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSchema {
    #[serde(rename = "type")]
    pub param_type: String,
    pub properties: Value,
    pub required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub tool_name: String,
    pub parameters: Value,
    pub expected_outcome: Option<String>,
}

#[async_trait]
pub trait TestTool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, params: Value) -> Result<Value>;
    async fn validate_result(&self, result: &Value) -> bool;
}

pub struct ToolRegistry {
    tools: std::collections::HashMap<String, Box<dyn TestTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: std::collections::HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn TestTool>) {
        let definition = tool.definition();
        self.tools.insert(definition.name.clone(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn TestTool> {
        self.tools.get(name).map(|boxed| &**boxed)
    }

    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|tool| tool.definition()).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
