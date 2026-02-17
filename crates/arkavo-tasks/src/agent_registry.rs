//! Agent registry for tracking available agents and their capabilities.

use crate::types::DeviceCapabilities;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Tracks available agents and their capabilities for task routing.
pub struct AgentRegistry {
    agents: Arc<RwLock<HashMap<String, AgentInfo>>>,
    capability_index: Arc<RwLock<HashMap<String, Vec<String>>>>, // capability -> [agent_ids]
}

/// Information about a registered agent.
#[derive(Debug, Clone)]
pub struct AgentInfo {
    /// Unique agent identifier
    pub agent_id: String,

    /// Agent display name
    pub name: String,

    /// Agent purpose/description
    pub purpose: String,

    /// List of capabilities this agent provides
    pub capabilities: Vec<String>,

    /// Device capabilities (for on-device agents like LocalAIAgent)
    pub device_caps: Option<DeviceCapabilities>,

    /// Additional metadata about the agent
    pub metadata: HashMap<String, String>,

    /// Last time this agent was seen/active
    pub last_seen: DateTime<Utc>,

    /// Current load (0.0 to 1.0), used for load balancing
    pub load: f32,

    /// Whether this agent is currently available
    pub is_available: bool,

    /// Network address if remote
    pub address: Option<String>,

    /// Base64-encoded ECDSA P-256 public key for TDF encryption
    pub public_key: Option<String>,
}

impl AgentRegistry {
    /// Create a new agent registry.
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
            capability_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register or update an agent with its capabilities.
    #[allow(clippy::too_many_arguments)]
    pub async fn register_agent(
        &self,
        agent_id: String,
        name: String,
        purpose: String,
        capabilities: Vec<String>,
        device_caps: Option<DeviceCapabilities>,
        metadata: HashMap<String, String>,
        address: Option<String>,
    ) -> Result<(), String> {
        info!(
            agent.id = %agent_id,
            capabilities = %capabilities.join(", "),
            "Registering agent"
        );

        let agent_info = AgentInfo {
            agent_id: agent_id.clone(),
            name,
            purpose,
            capabilities: capabilities.clone(),
            device_caps,
            metadata,
            last_seen: Utc::now(),
            load: 0.0,
            is_available: true,
            address,
            public_key: None,
        };

        // Store agent info
        {
            let mut agents = self.agents.write().await;
            agents.insert(agent_id.clone(), agent_info);
        }

        // Update capability index
        {
            let mut index = self.capability_index.write().await;

            // Add this agent to all its capabilities
            for capability in &capabilities {
                index
                    .entry(capability.clone())
                    .or_insert_with(Vec::new)
                    .push(agent_id.clone());
            }
        }

        debug!(
            agent.id = %agent_id,
            capabilities = %capabilities.len(),
            "Agent registered successfully"
        );

        Ok(())
    }

    /// Unregister an agent.
    pub async fn unregister_agent(&self, agent_id: &str) -> Result<(), String> {
        info!(agent.id = %agent_id, "Unregistering agent");

        // Get agent's capabilities before removing
        let capabilities = {
            let agents = self.agents.read().await;
            agents
                .get(agent_id)
                .map(|info| info.capabilities.clone())
                .ok_or_else(|| format!("Agent {agent_id} not found"))?
        };

        // Remove from agent list
        {
            let mut agents = self.agents.write().await;
            agents.remove(agent_id);
        }

        // Remove from capability index
        {
            let mut index = self.capability_index.write().await;
            for capability in &capabilities {
                if let Some(agent_ids) = index.get_mut(capability) {
                    agent_ids.retain(|id| id != agent_id);
                    if agent_ids.is_empty() {
                        index.remove(capability);
                    }
                }
            }
        }

        Ok(())
    }

    /// Find agents that have a specific capability.
    pub async fn find_agents_with_capability(&self, capability: &str) -> Vec<String> {
        let index = self.capability_index.read().await;
        index.get(capability).cloned().unwrap_or_default()
    }

    /// Find the best agent for a specific capability (considering load).
    ///
    /// # Panics
    /// Panics if load values cannot be compared (e.g., NaN values).
    pub async fn find_best_agent(&self, capability: &str) -> Option<String> {
        let candidate_ids = self.find_agents_with_capability(capability).await;

        if candidate_ids.is_empty() {
            warn!(capability = %capability, "No agents found with capability");
            return None;
        }

        // Get agent infos and filter by availability
        let agents = self.agents.read().await;
        let mut available_agents: Vec<_> = candidate_ids
            .iter()
            .filter_map(|id| agents.get(id))
            .filter(|info| info.is_available)
            .collect();

        if available_agents.is_empty() {
            warn!(capability = %capability, "No available agents found");
            return None;
        }

        // Sort by load (ascending) to get least loaded agent
        // Handle NaN values by treating them as greater than any valid load
        available_agents.sort_by(|a, b| {
            a.load.partial_cmp(&b.load).unwrap_or_else(|| {
                // If either is NaN, sort NaN values to the end
                if a.load.is_nan() && b.load.is_nan() {
                    std::cmp::Ordering::Equal
                } else if a.load.is_nan() {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Less
                }
            })
        });

        let best_agent = available_agents[0].agent_id.clone();
        debug!(
            capability = %capability,
            agent.id = %best_agent,
            load = %available_agents[0].load,
            "Selected best agent"
        );

        Some(best_agent)
    }

    /// Find agents that have ALL of the specified capabilities.
    pub async fn find_agents_with_all_capabilities(&self, capabilities: &[String]) -> Vec<String> {
        if capabilities.is_empty() {
            return Vec::new();
        }

        // Get agent IDs for first capability
        let mut result = self.find_agents_with_capability(&capabilities[0]).await;

        // Intersect with agents for each remaining capability
        for capability in &capabilities[1..] {
            let agents_with_cap = self.find_agents_with_capability(capability).await;
            result.retain(|agent_id| agents_with_cap.contains(agent_id));
        }

        result
    }

    /// Update an agent's load metric.
    pub async fn update_agent_load(&self, agent_id: &str, load: f32) -> Result<(), String> {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(agent_id) {
            // Normalize non-finite values before clamping (clamp preserves NaN)
            let safe_load = if load.is_finite() { load } else { 1.0 };
            agent.load = safe_load.clamp(0.0, 1.0);
            agent.last_seen = Utc::now();
            Ok(())
        } else {
            Err(format!("Agent {agent_id} not found"))
        }
    }

    /// Mark an agent as available or unavailable.
    pub async fn set_agent_availability(
        &self,
        agent_id: &str,
        available: bool,
    ) -> Result<(), String> {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(agent_id) {
            agent.is_available = available;
            agent.last_seen = Utc::now();
            info!(
                agent.id = %agent_id,
                available = available,
                "Agent availability updated"
            );
            Ok(())
        } else {
            Err(format!("Agent {agent_id} not found"))
        }
    }

    /// Get information about a specific agent.
    pub async fn get_agent_info(&self, agent_id: &str) -> Option<AgentInfo> {
        let agents = self.agents.read().await;
        agents.get(agent_id).cloned()
    }

    /// Get all registered agents.
    pub async fn get_all_agents(&self) -> Vec<AgentInfo> {
        let agents = self.agents.read().await;
        agents.values().cloned().collect()
    }

    /// Get all available capabilities.
    pub async fn get_all_capabilities(&self) -> Vec<String> {
        let index = self.capability_index.read().await;
        index.keys().cloned().collect()
    }

    /// Cleanup stale agents (not seen for a while).
    pub async fn cleanup_stale_agents(&self, max_age_seconds: i64) {
        let now = Utc::now();
        let mut agents_to_remove = Vec::new();

        {
            let agents = self.agents.read().await;
            for (agent_id, info) in agents.iter() {
                let age = (now - info.last_seen).num_seconds();
                if age > max_age_seconds {
                    agents_to_remove.push(agent_id.clone());
                }
            }
        }

        for agent_id in agents_to_remove {
            warn!(
                agent.id = %agent_id,
                "Removing stale agent (inactive for {} seconds)",
                max_age_seconds
            );
            let _ = self.unregister_agent(&agent_id).await;
        }
    }

    /// Heartbeat - update agent's last_seen timestamp.
    pub async fn heartbeat(&self, agent_id: &str) -> Result<(), String> {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(agent_id) {
            agent.last_seen = Utc::now();
            debug!(agent.id = %agent_id, "Agent heartbeat received");
            Ok(())
        } else {
            Err(format!("Agent {agent_id} not found"))
        }
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_and_find_agent() {
        let registry = AgentRegistry::new();

        // Register agent with sensor capability
        registry
            .register_agent(
                "local_ai_1".to_string(),
                "LocalAI".to_string(),
                "On-device AI".to_string(),
                vec!["sensors".to_string(), "foundation_models".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        // Find agents with sensor capability
        let agents = registry.find_agents_with_capability("sensors").await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0], "local_ai_1");
    }

    #[tokio::test]
    async fn test_find_best_agent_by_load() {
        let registry = AgentRegistry::new();

        // Register two agents with same capability but different load
        registry
            .register_agent(
                "agent_1".to_string(),
                "Agent 1".to_string(),
                "Test".to_string(),
                vec!["search".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        registry
            .register_agent(
                "agent_2".to_string(),
                "Agent 2".to_string(),
                "Test".to_string(),
                vec!["search".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        // Update load
        registry.update_agent_load("agent_1", 0.8).await.unwrap();
        registry.update_agent_load("agent_2", 0.2).await.unwrap();

        // Best agent should be agent_2 (lower load)
        let best = registry.find_best_agent("search").await;
        assert_eq!(best, Some("agent_2".to_string()));
    }

    #[tokio::test]
    async fn test_find_best_agent_with_nan_load() {
        // This test verifies that NaN load values don't cause a panic
        let registry = AgentRegistry::new();

        // Register two agents with same capability
        registry
            .register_agent(
                "agent_1".to_string(),
                "Agent 1".to_string(),
                "Test".to_string(),
                vec!["search".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        registry
            .register_agent(
                "agent_2".to_string(),
                "Agent 2".to_string(),
                "Test".to_string(),
                vec!["search".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        // NaN load is normalized to 1.0 (max load) at the source
        registry
            .update_agent_load("agent_1", f32::NAN)
            .await
            .unwrap();
        registry.update_agent_load("agent_2", 0.5).await.unwrap();

        // agent_1 has load 1.0 (normalized from NaN), agent_2 has load 0.5
        let best = registry.find_best_agent("search").await;
        assert_eq!(best, Some("agent_2".to_string()));
    }

    #[tokio::test]
    async fn test_find_best_agent_with_infinite_load() {
        // This test verifies that infinite load values are handled correctly
        let registry = AgentRegistry::new();

        registry
            .register_agent(
                "agent_1".to_string(),
                "Agent 1".to_string(),
                "Test".to_string(),
                vec!["search".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        registry
            .register_agent(
                "agent_2".to_string(),
                "Agent 2".to_string(),
                "Test".to_string(),
                vec!["search".to_string()],
                None,
                HashMap::new(),
                None,
            )
            .await
            .unwrap();

        // Update loads with infinity
        registry
            .update_agent_load("agent_1", f32::INFINITY)
            .await
            .unwrap();
        registry.update_agent_load("agent_2", 0.5).await.unwrap();

        // Best agent should be agent_2 (finite load is less than infinity)
        let best = registry.find_best_agent("search").await;
        assert_eq!(best, Some("agent_2".to_string()));
    }
}
