use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Permit,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityIdentifier {
    pub id: String,
    #[serde(rename = "type")]
    pub entity_type: EntityType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attributes: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityType {
    ClientId,
    Email,
    Username,
    Subject,
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resource {
    pub attribute_value_fqns: Vec<String>,
}

impl Resource {
    pub fn new(fqns: Vec<String>) -> Self {
        Self {
            attribute_value_fqns: fqns,
        }
    }

    pub fn mcp_tool(tool_name: &str) -> Self {
        Self {
            attribute_value_fqns: vec![format!(
                "https://arkavo.net/attr/mcp-tool/value/{}",
                tool_name
            )],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
}

impl Action {
    pub fn execute_tool() -> Self {
        Self {
            name: "execute_tool".to_string(),
        }
    }

    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }
}

// Entity Resolution v2 types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEntityChainsFromTokensRequest {
    pub tokens: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEntityChainsFromTokensResponse {
    pub entity_chains: Vec<EntityChain>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityChain {
    pub id: String,
    pub entities: Vec<EntityIdentifier>,
}

// Authorization v2 - GetDecision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDecisionRequest {
    pub entity_identifier: EntityIdentifier,
    pub action: Action,
    pub resource: Resource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDecisionResponse {
    pub decision: Decision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obligations: Option<Vec<String>>,
}

// Authorization v2 - GetDecisionMultiResource
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDecisionMultiResourceRequest {
    pub entity_identifier: EntityIdentifier,
    pub action: Action,
    pub resources: Vec<Resource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDecisionMultiResourceResponse {
    pub decisions: Vec<ResourceDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceDecision {
    pub resource: Resource,
    pub decision: Decision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obligations: Option<Vec<String>>,
}

// Authorization v2 - GetDecisionBulk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDecisionBulkRequest {
    pub decision_requests: Vec<DecisionRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub entity_identifier: EntityIdentifier,
    pub action: Action,
    pub resource: Resource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetDecisionBulkResponse {
    pub decision_responses: Vec<DecisionResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionResponse {
    pub entity_identifier: EntityIdentifier,
    pub action: Action,
    pub resource: Resource,
    pub decision: Decision,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obligations: Option<Vec<String>>,
}

// MCP Tool mapping helpers
pub struct McpToolMapping;

impl McpToolMapping {
    pub fn tool_to_resource(tool_name: &str) -> Resource {
        let fqn = match tool_name {
            "filesystem_read" => "https://arkavo.net/attr/mcp-tool/value/filesystem.read",
            "filesystem_write" => "https://arkavo.net/attr/mcp-tool/value/filesystem.write",
            "git_commit" => "https://arkavo.net/attr/mcp-tool/value/git.commit",
            "git_push" => "https://arkavo.net/attr/mcp-tool/value/git.push",
            "device_tap" => "https://arkavo.net/attr/mcp-tool/value/device_management.tap",
            "device_swipe" => "https://arkavo.net/attr/mcp-tool/value/device_management.swipe",
            _ => &format!("https://arkavo.net/attr/mcp-tool/value/{tool_name}"),
        };
        Resource::new(vec![fqn.to_string()])
    }

    pub fn is_safe_diagnostic(tool_name: &str) -> bool {
        matches!(tool_name, "status" | "health" | "version" | "list_tools")
    }
}
