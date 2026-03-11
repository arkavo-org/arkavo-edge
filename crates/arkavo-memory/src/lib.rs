pub mod advisor_state;
pub mod case_retrieval;
pub mod embeddings;
pub mod encryption;
pub mod error;
pub mod event_store;
pub mod federated_memory;
pub mod ledger;
#[cfg(feature = "vector-search")]
pub mod mcp_tools;
pub mod memory_lifecycle;
pub mod models;
pub mod orchestrator_state;
pub mod plan_state;
#[cfg(feature = "vector-search")]
pub mod storage;
pub mod tdf_audit_store;
pub mod workspace_config;

pub use advisor_state::{AdvisorStateStore, PersistedAdjustment};
pub use case_retrieval::{CaseIndex, CaseMatch, IndexMetadata};
pub use federated_memory::{
    ContextManifest, FederatedItem, FederatedMemoryService, FederatedQuery, FederatedResult,
    MemoryAttribute, MemoryPolicy, evaluate_entitlements,
};
pub use ledger::ContextLedger;
pub use memory_lifecycle::{LifecycleConfig, LifecycleReport, LifecycleRules};
pub use models::{AgentConversation, AgentDomain, CreateMemoryRequest, Memory, SearchResult};
pub use orchestrator_state::{
    IssueProcessingStatus, OrchestratorStateStore, OrgStats, ProcessedIssue, RepoState, RepoStatus,
};
pub use plan_state::{PersistedPlan, PlanStateStore, PlanStatus};
#[cfg(feature = "vector-search")]
pub use storage::{HnswConfig, MemoryStorage};
pub use tdf_audit_store::{AuditRecord, TdfAuditStore};
pub use workspace_config::WorkspaceConfig;
