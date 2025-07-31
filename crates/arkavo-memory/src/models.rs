use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMemoryRequest {
    pub content: String,
    pub metadata: Option<Value>,
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    pub content: String,
    pub metadata: Option<Value>,
    pub category: Option<String>,
    pub embedding: Vec<f32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub memory: Memory,
    pub score: f32,
}

/// Represents an agent-to-agent conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConversation {
    pub id: Uuid,
    pub from_agent_id: String,
    pub to_agent_id: String,
    pub query: String,
    pub response: String,
    pub confidence: f32,
    pub domain: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Predefined agent domain categories
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentDomain {
    Security,
    CodeReview,
    Database,
    Testing,
    Documentation,
    Performance,
    DevOps,
    Frontend,
    Architecture,
    DataScience,
    Orchestration,
    Custom(String),
}

impl AgentDomain {
    pub fn as_str(&self) -> &str {
        match self {
            AgentDomain::Security => "security",
            AgentDomain::CodeReview => "code_review",
            AgentDomain::Database => "database",
            AgentDomain::Testing => "testing",
            AgentDomain::Documentation => "documentation",
            AgentDomain::Performance => "performance",
            AgentDomain::DevOps => "devops",
            AgentDomain::Frontend => "frontend",
            AgentDomain::Architecture => "architecture",
            AgentDomain::DataScience => "data_science",
            AgentDomain::Orchestration => "orchestration",
            AgentDomain::Custom(s) => s,
        }
    }
}
