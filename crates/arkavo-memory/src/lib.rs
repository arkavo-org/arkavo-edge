pub mod embeddings;
pub mod error;
pub mod event_store;
pub mod ledger;
#[cfg(feature = "vector-search")]
pub mod mcp_tools;
pub mod models;
pub mod orchestrator_state;
#[cfg(feature = "vector-search")]
pub mod storage;

pub use ledger::ContextLedger;
pub use models::{AgentConversation, AgentDomain, CreateMemoryRequest, Memory, SearchResult};
pub use orchestrator_state::{
    IssueProcessingStatus, OrchestratorStateStore, OrgStats, ProcessedIssue, RepoState, RepoStatus,
};
#[cfg(feature = "vector-search")]
pub use storage::{HnswConfig, MemoryStorage};
