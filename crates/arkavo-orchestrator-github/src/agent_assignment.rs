use crate::error::{Error, Result};
use crate::issue_router::RoutingDecision;
use crate::types::IssueEvent;
use arkavo_protocol::agent_registry::AgentRegistry;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAssignment {
    pub issue_number: u64,
    pub repository: String,
    pub issue_title: String,
    pub issue_body: String,
    pub assigned_agent_id: Option<String>,
    pub routing_decision: RoutingDecision,
    pub assignment_rationale: String,
}

pub struct AgentAssigner {
    registry: Arc<AgentRegistry>,
}

impl AgentAssigner {
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self { registry }
    }

    pub async fn assign(
        &self,
        event: &IssueEvent,
        decision: RoutingDecision,
    ) -> Result<AgentAssignment> {
        info!(
            issue_number = event.issue.number,
            repository = %event.repository.full_name,
            strategy = ?decision.strategy,
            "Assigning issue to agent"
        );

        let required_capabilities = &decision.analysis.required_capabilities;

        if required_capabilities.is_empty() {
            warn!("No required capabilities determined for issue");
            return Ok(AgentAssignment {
                issue_number: event.issue.number,
                repository: event.repository.full_name.clone(),
                issue_title: event.issue.title.clone(),
                issue_body: event.issue.body.clone().unwrap_or_default(),
                assigned_agent_id: None,
                routing_decision: decision,
                assignment_rationale: "No specific capabilities required - will use default agent"
                    .to_string(),
            });
        }

        let mut assigned_agent_id = None;
        let mut assignment_rationale = String::new();

        for capability in required_capabilities {
            debug!(capability, "Looking for agent with capability");

            match self.registry.find_best_agent(capability).await {
                Some(agent_id) => {
                    info!(agent_id, capability, "Found agent with required capability");
                    assigned_agent_id = Some(agent_id.clone());
                    assignment_rationale =
                        format!("Assigned to agent {agent_id} based on capability: {capability}");
                    break;
                }
                None => {
                    debug!(capability, "No agent found with capability");
                    continue;
                }
            }
        }

        if assigned_agent_id.is_none() {
            assignment_rationale = format!(
                "No agent found with required capabilities: {}. Will require manual assignment or agent registration.",
                required_capabilities.join(", ")
            );
        }

        Ok(AgentAssignment {
            issue_number: event.issue.number,
            repository: event.repository.full_name.clone(),
            issue_title: event.issue.title.clone(),
            issue_body: event.issue.body.clone().unwrap_or_default(),
            assigned_agent_id,
            routing_decision: decision,
            assignment_rationale,
        })
    }

    pub async fn find_multi_agent_team(
        &self,
        required_capabilities: &[String],
    ) -> Result<Vec<String>> {
        let mut team = Vec::new();
        let mut unique_agents = std::collections::HashSet::new();

        for capability in required_capabilities {
            if let Some(agent_id) = self.registry.find_best_agent(capability).await
                && unique_agents.insert(agent_id.clone())
            {
                team.push(agent_id);
            }
        }

        if team.is_empty() {
            return Err(Error::Other(anyhow::anyhow!(
                "No agents available for required capabilities"
            )));
        }

        Ok(team)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[allow(clippy::disallowed_methods)]
    async fn test_agent_assignment_creation() {
        let registry = Arc::new(AgentRegistry::new());
        let assigner = AgentAssigner::new(registry);

        assert!(matches!(assigner.find_multi_agent_team(&[]).await, Err(_)));
    }
}
