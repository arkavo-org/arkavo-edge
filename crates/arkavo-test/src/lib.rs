#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::significant_drop_in_scrutinee)]
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]

pub mod ai;
pub mod bridge;
pub mod execution;
pub mod gherkin;
pub mod integration;
pub mod mcp;
pub mod reporting;
pub mod state_store;

#[cfg(test)]
mod state_store_test;

#[cfg(feature = "memory")]
use arkavo_memory::error::MemoryError;
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TestError {
    #[error("MCP error: {0}")]
    Mcp(String),

    #[error("Gherkin parsing error: {0}")]
    GherkinParse(String),

    #[error("Execution error: {0}")]
    Execution(String),

    #[error("Bridge error: {0}")]
    Bridge(String),

    #[error("AI error: {0}")]
    Ai(String),

    #[error("Reporting error: {0}")]
    Reporting(String),

    #[cfg(feature = "memory")]
    #[error("Memory error: {0}")]
    Memory(#[from] MemoryError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, TestError>;

#[derive(Debug, Clone)]
pub struct TestHarness {
    mcp_server: Arc<mcp::server::McpTestServer>,
    state_manager: Arc<execution::state::StateManager>,
}

impl TestHarness {
    pub fn new() -> Result<Self> {
        Ok(Self {
            mcp_server: Arc::new(mcp::server::McpTestServer::new()?),
            state_manager: Arc::new(execution::state::StateManager::new()?),
        })
    }

    pub const fn mcp_server(&self) -> &Arc<mcp::server::McpTestServer> {
        &self.mcp_server
    }

    pub const fn state_manager(&self) -> &Arc<execution::state::StateManager> {
        &self.state_manager
    }

    pub fn state_store(&self) -> Arc<state_store::StateStore> {
        self.mcp_server.state_store().clone()
    }
}
