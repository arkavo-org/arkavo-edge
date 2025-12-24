//! InjectHazard MCP Tool
//!
//! Injects a hazard into a specific sector.

use crate::{FleetEnvState, Hazard};
use arkavo_mcp::{Tool, ToolSchema};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Tool for injecting hazards into sectors
pub struct InjectHazardTool {
    schema: ToolSchema,
    state: Arc<FleetEnvState>,
}

impl InjectHazardTool {
    pub fn new(state: Arc<FleetEnvState>) -> Self {
        Self {
            schema: ToolSchema {
                name: "inject_hazard".to_string(),
                aliases: Some(vec!["add_hazard".to_string(), "create_hazard".to_string()]),
                description: "Inject a hazard into a warehouse sector".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "sector": {
                            "type": "integer",
                            "description": "Sector ID to inject hazard into"
                        },
                        "hazard_type": {
                            "type": "string",
                            "description": "Type of hazard (e.g., 'black_ice', 'debris', 'flood')"
                        },
                        "traction": {
                            "type": "number",
                            "description": "Traction coefficient (0.0 = no traction, 1.0 = full)"
                        }
                    },
                    "required": ["sector", "hazard_type", "traction"]
                }),
            },
            state,
        }
    }
}

#[async_trait]
impl Tool for InjectHazardTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let sector = params
            .get("sector")
            .and_then(|v| v.as_u64())
            .map(|v| v as u8)
            .ok_or_else(|| "Missing or invalid 'sector' parameter")?;

        let hazard_type = params
            .get("hazard_type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing or invalid 'hazard_type' parameter")?
            .to_string();

        let traction = params
            .get("traction")
            .and_then(|v| v.as_f64())
            .map(|v| v as f32)
            .ok_or_else(|| "Missing or invalid 'traction' parameter")?;

        let hazard = Hazard {
            hazard_type: hazard_type.clone(),
            traction,
        };

        if self.state.inject_hazard(sector, hazard).await {
            tracing::warn!(
                sector_id = sector,
                hazard_type = %hazard_type,
                traction = traction,
                "Hazard injected"
            );
            Ok(json!({
                "success": true,
                "sector": sector,
                "hazard_type": hazard_type,
                "traction": traction
            }))
        } else {
            Ok(json!({
                "success": false,
                "error": format!("Sector {} not found", sector)
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_inject_hazard_tool() {
        let state = Arc::new(FleetEnvState::new(4));
        let tool = InjectHazardTool::new(Arc::clone(&state));

        let result = tool
            .execute(json!({
                "sector": 4,
                "hazard_type": "black_ice",
                "traction": 0.2
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["sector"], 4);

        // Verify hazard was injected
        let sector = state.get_sector(4).await.unwrap();
        assert!(sector.hazard.is_some());
    }

    #[tokio::test]
    async fn test_inject_hazard_nonexistent_sector() {
        let state = Arc::new(FleetEnvState::new(4));
        let tool = InjectHazardTool::new(state);

        let result = tool
            .execute(json!({
                "sector": 99,
                "hazard_type": "flood",
                "traction": 0.1
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], false);
    }
}
