//! Runtime implementation for Model Context Protocol (MCP) server
//!
//! This crate provides the server implementation and built-in tools for MCP.

pub mod server;
pub mod tools;

// Re-export core types for convenience
pub use arkavo_mcp_core::{
    RpcError, RpcRequest, RpcResponse, Tool, ToolRequest, ToolResponse, ToolSchema, error_codes,
};

pub use server::McpServer;
