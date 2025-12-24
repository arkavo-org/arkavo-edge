//! Fleet Environment MCP Tools
//!
//! Standalone MCP tools for simulating a fleet environment with sectors
//! that can have hazards. Used by the fleet-immunity example.

mod get_sector;
mod inject_hazard;

pub use get_sector::GetSectorTool;
pub use inject_hazard::InjectHazardTool;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::RwLock;

/// MCP Tool schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    pub description: String,
    pub parameters: Value,
}

/// MCP Tool trait
#[async_trait]
pub trait Tool: Send + Sync {
    async fn execute(
        &self,
        params: Value,
    ) -> Result<Value, Box<dyn std::error::Error + Send + Sync>>;
    fn schema(&self) -> &ToolSchema;
}

/// Represents a hazard in a sector
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hazard {
    pub hazard_type: String,
    pub traction: f32,
}

/// Represents a sector in the warehouse/environment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sector {
    pub id: u8,
    pub name: String,
    pub hazard: Option<Hazard>,
}

/// Shared state for the fleet environment
pub struct FleetEnvState {
    pub sectors: RwLock<HashMap<u8, Sector>>,
}

impl FleetEnvState {
    pub fn new(sector_count: u8) -> Self {
        let mut sectors = HashMap::new();
        let names = ["Loading Dock", "Main Aisle", "Storage A", "Cold Storage"];
        for i in 1..=sector_count {
            sectors.insert(
                i,
                Sector {
                    id: i,
                    name: names.get((i - 1) as usize).unwrap_or(&"Sector").to_string(),
                    hazard: None,
                },
            );
        }
        Self {
            sectors: RwLock::new(sectors),
        }
    }

    pub async fn get_sector(&self, id: u8) -> Option<Sector> {
        self.sectors.read().await.get(&id).cloned()
    }

    pub async fn inject_hazard(&self, sector_id: u8, hazard: Hazard) -> bool {
        let mut sectors = self.sectors.write().await;
        if let Some(sector) = sectors.get_mut(&sector_id) {
            sector.hazard = Some(hazard);
            true
        } else {
            false
        }
    }

    pub async fn clear_hazard(&self, sector_id: u8) -> bool {
        let mut sectors = self.sectors.write().await;
        if let Some(sector) = sectors.get_mut(&sector_id) {
            sector.hazard = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_state() {
        let state = FleetEnvState::new(4);
        assert_eq!(state.sectors.read().await.len(), 4);
    }

    #[tokio::test]
    async fn test_get_sector() {
        let state = FleetEnvState::new(4);
        let sector = state.get_sector(1).await;
        assert!(sector.is_some());
        assert_eq!(sector.unwrap().id, 1);
    }

    #[tokio::test]
    async fn test_inject_hazard() {
        let state = FleetEnvState::new(4);
        let hazard = Hazard {
            hazard_type: "black_ice".to_string(),
            traction: 0.2,
        };
        assert!(state.inject_hazard(4, hazard).await);
        let sector = state.get_sector(4).await.unwrap();
        assert!(sector.hazard.is_some());
        assert_eq!(sector.hazard.unwrap().hazard_type, "black_ice");
    }
}
