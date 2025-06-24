use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[async_trait]
pub trait Tool: Send + Sync {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>>;
    fn schema(&self) -> &ToolSchema;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolRequest {
    pub tool_name: String,
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolResponse {
    pub tool_name: String,
    pub result: Value,
    pub success: bool,
}
