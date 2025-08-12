#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::significant_drop_in_scrutinee)]
#![allow(clippy::pedantic)]
#![allow(clippy::nursery)]

// This crate is macOS-only because it contains iOS simulator and XCTest functionality
// On non-macOS platforms, this crate provides no functionality

// Provide a doc comment for non-macOS platforms
#[cfg(not(target_os = "macos"))]
/// This crate only provides functionality on macOS.
/// For cross-platform MCP tools, use the `arkavo-mcp-tools` crate instead.
pub struct NotAvailableOnThisPlatform;

#[cfg(target_os = "macos")]
pub mod ai;
#[cfg(target_os = "macos")]
pub mod bridge;
#[cfg(target_os = "macos")]
pub mod execution;
#[cfg(target_os = "macos")]
pub mod gherkin;
#[cfg(target_os = "macos")]
pub mod integration;
#[cfg(target_os = "macos")]
pub mod mcp;
#[cfg(target_os = "macos")]
pub mod reporting;
#[cfg(target_os = "macos")]
pub mod state_store;

#[cfg(all(test, target_os = "macos"))]
mod state_store_test;

#[cfg(target_os = "macos")]
#[cfg(feature = "memory")]
use arkavo_memory::error::MemoryError;
#[cfg(target_os = "macos")]
use std::sync::Arc;
#[cfg(target_os = "macos")]
use thiserror::Error;

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
pub type Result<T> = std::result::Result<T, TestError>;

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
pub struct TestHarness {
    mcp_server: Arc<mcp::server::McpTestServer>,
    state_manager: Arc<execution::state::StateManager>,
}

#[cfg(target_os = "macos")]
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
