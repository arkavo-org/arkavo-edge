pub mod embeddings;
pub mod error;
pub mod event_store;
#[cfg(feature = "vector-search")]
pub mod mcp_tools;
pub mod models;
#[cfg(feature = "vector-search")]
pub mod storage;

pub use models::{AgentConversation, AgentDomain, CreateMemoryRequest, Memory, SearchResult};
#[cfg(feature = "vector-search")]
pub use storage::MemoryStorage;
