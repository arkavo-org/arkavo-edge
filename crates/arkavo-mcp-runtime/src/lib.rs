//! Runtime implementation for Model Context Protocol (MCP) server
//!
//! This crate provides the server implementation and built-in tools for MCP,
//! as well as a dynamic runtime for managing MCP client connections.

pub mod client;
pub mod runtime;
pub mod server;
pub mod tools;

// Re-export core types for convenience
pub use arkavo_mcp_core::{
    RpcError, RpcRequest, RpcResponse, Tool, ToolRequest, ToolResponse, ToolSchema, error_codes,
};

pub use client::{ClientHandle, McpServerConfig, McpTransportConfig, ToolInfo};
pub use runtime::McpRuntime;
pub use server::McpServer;
