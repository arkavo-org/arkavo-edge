//! GetSector MCP Tool
//!
//! Queries sector information including current hazard status.

use crate::FleetEnvState;
use arkavo_mcp::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool for querying sector information
pub struct GetSectorTool {
    schema: ToolSchema,
    state: Arc<FleetEnvState>,
}

impl GetSectorTool {
    pub fn new(state: Arc<FleetEnvState>) -> Self {
        Self {
            schema: ToolSchema {
                name: "get_sector".to_string(),
                aliases: Some(vec!["sector_info".to_string(), "query_sector".to_string()]),
                description: "Get information about a warehouse sector, including any hazards"
                    .to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Sector ID (1-based)"
                        }
                    },
                    "required": ["id"]
                }),
            },
            state,
        }
    }
}

#[async_trait]
impl Tool for GetSectorTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let id = params
            .get("id")
            .and_then(|v| v.as_u64())
            .map(|v| v as u8)
            .ok_or_else(|| "Missing or invalid 'id' parameter")?;

        match self.state.get_sector(id).await {
            Some(sector) => {
                tracing::info!(sector_id = id, "Queried sector");
                Ok(json!({
                    "id": sector.id,
                    "name": sector.name,
                    "hazard": sector.hazard.map(|h| json!({
                        "type": h.hazard_type,
                        "traction": h.traction
                    }))
                }))
            }
            None => Ok(json!({
                "error": format!("Sector {} not found", id)
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_sector_tool() {
        let state = Arc::new(FleetEnvState::new(4));
        let tool = GetSectorTool::new(state);

        let result = tool.execute(json!({"id": 1})).await.unwrap();
        assert_eq!(result["id"], 1);
        assert_eq!(result["name"], "Sector 1");
        assert!(result["hazard"].is_null());
    }

    #[tokio::test]
    async fn test_get_sector_with_hazard() {
        let state = Arc::new(FleetEnvState::new(4));
        state
            .inject_hazard(
                2,
                crate::Hazard {
                    hazard_type: "ice".to_string(),
                    traction: 0.3,
                },
            )
            .await;

        let tool = GetSectorTool::new(state);
        let result = tool.execute(json!({"id": 2})).await.unwrap();

        assert!(result["hazard"].is_object());
        assert_eq!(result["hazard"]["type"], "ice");
    }

    #[tokio::test]
    async fn test_get_nonexistent_sector() {
        let state = Arc::new(FleetEnvState::new(4));
        let tool = GetSectorTool::new(state);

        let result = tool.execute(json!({"id": 99})).await.unwrap();
        assert!(result["error"].is_string());
    }
}
