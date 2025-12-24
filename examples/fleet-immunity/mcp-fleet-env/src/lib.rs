//! Fleet Environment MCP Tools
//!
//! This crate provides MCP tools for simulating a fleet environment with
//! sectors that can have hazards injected. Used by the fleet-immunity example
//! to demonstrate agent learning and coordination.
//!
//! ## Tools
//!
//! - `get_sector` - Query sector information including hazard status
//! - `inject_hazard` - Inject a hazard into a specific sector
//!
//! ## Usage
//!
//! ```ignore
//! use arkavo_mcp_fleet_env::{FleetEnvState, register_tools};
//! use std::sync::Arc;
//!
//! let state = Arc::new(FleetEnvState::new(4)); // 4 sectors
//! register_tools(&mut registry, state);
//! ```

mod get_sector;
mod inject_hazard;

pub use get_sector::GetSectorTool;
pub use inject_hazard::InjectHazardTool;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Represents a hazard in a sector
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hazard {
    /// Type of hazard (e.g., "black_ice", "debris", "flood")
    pub hazard_type: String,
    /// Traction coefficient (0.0 = no traction, 1.0 = full traction)
    pub traction: f32,
}

/// Represents a sector in the warehouse/environment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sector {
    /// Sector ID
    pub id: u8,
    /// Human-readable name
    pub name: String,
    /// Current hazard, if any
    pub hazard: Option<Hazard>,
}

/// Shared state for the fleet environment
pub struct FleetEnvState {
    /// All sectors indexed by ID
    pub sectors: RwLock<HashMap<u8, Sector>>,
}

impl FleetEnvState {
    /// Create a new fleet environment with the specified number of sectors
    pub fn new(sector_count: u8) -> Self {
        let mut sectors = HashMap::new();
        for i in 1..=sector_count {
            sectors.insert(
                i,
                Sector {
                    id: i,
                    name: format!("Sector {}", i),
                    hazard: None,
                },
            );
        }
        Self {
            sectors: RwLock::new(sectors),
        }
    }

    /// Get a sector by ID
    pub async fn get_sector(&self, id: u8) -> Option<Sector> {
        self.sectors.read().await.get(&id).cloned()
    }

    /// Inject a hazard into a sector
    pub async fn inject_hazard(&self, sector_id: u8, hazard: Hazard) -> bool {
        let mut sectors = self.sectors.write().await;
        if let Some(sector) = sectors.get_mut(&sector_id) {
            sector.hazard = Some(hazard);
            true
        } else {
            false
        }
    }

    /// Clear hazard from a sector
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

/// Register fleet environment tools with a tool registry
pub fn register_tools(registry: &mut dyn ToolRegistry, state: Arc<FleetEnvState>) {
    registry.register(Box::new(GetSectorTool::new(Arc::clone(&state))));
    registry.register(Box::new(InjectHazardTool::new(state)));
}

/// Trait for tool registry (re-export pattern)
pub trait ToolRegistry {
    fn register(&mut self, tool: Box<dyn arkavo_mcp::Tool>);
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

    #[tokio::test]
    async fn test_clear_hazard() {
        let state = FleetEnvState::new(4);

        let hazard = Hazard {
            hazard_type: "debris".to_string(),
            traction: 0.5,
        };

        state.inject_hazard(2, hazard).await;
        assert!(state.get_sector(2).await.unwrap().hazard.is_some());

        state.clear_hazard(2).await;
        assert!(state.get_sector(2).await.unwrap().hazard.is_none());
    }
}
