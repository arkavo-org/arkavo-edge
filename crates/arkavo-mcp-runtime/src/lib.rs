//! Runtime implementation for Model Context Protocol (MCP) server
//!
//! This crate provides the server implementation, client management, and built-in tools for MCP.

#![allow(clippy::significant_drop_in_scrutinee)]
#![allow(clippy::significant_drop_tightening)]

pub mod client;
pub mod polling;
pub mod runtime;
pub mod server;
pub mod tools;
pub mod transport;

// Re-export core types for convenience
pub use arkavo_mcp::{
    McpClient as McpClientTrait, McpTool, RpcError, RpcNotification, RpcRequest, RpcResponse, Tool,
    ToolRequest, ToolResponse, ToolSchema, error_codes,
};

pub use client::{ClientHandle, McpClient, McpServerConfig};
pub use polling::{AdaptiveBackoff, PollConfig, PollResultParams, PollableEndpoint};
pub use runtime::{McpRuntime, RuntimeError};
pub use server::McpServer;
pub use transport::{
    SseTransport, StdioTransport, Transport, TransportConfig, TransportError, WebSocketTransport,
    create_transport,
};
