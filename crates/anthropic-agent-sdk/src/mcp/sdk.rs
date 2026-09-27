//! SDK MCP Server support via rmcp
//!
//! This module provides re-exports from the official rmcp crate for creating
//! in-process MCP servers with custom tools.
//!
//! # Example
//!
//! ```ignore
//! use anthropic_agent_sdk::mcp::{tool, tool_router, tool_handler};
//! use anthropic_agent_sdk::mcp::{Parameters, CallToolResult, ContentBlock, ToolRouter, ServerHandler};
//! use anthropic_agent_sdk::mcp::{Implementation, ServerCapabilities, ServerConfig};
//! use schemars::JsonSchema;
//! use serde::Deserialize;
//!
//! #[derive(Deserialize, JsonSchema)]
//! struct GreetParams {
//!     name: String,
//! }
//!
//! #[derive(Clone)]
//! struct Greeter {
//!     tool_router: ToolRouter<Self>,
//! }
//!
//! #[tool_router]
//! impl Greeter {
//!     fn new() -> Self {
//!         Self { tool_router: Self::tool_router() }
//!     }
//!
//!     #[tool(description = "Greet someone by name")]
//!     async fn greet(&self, params: Parameters<GreetParams>) -> Result<CallToolResult, String> {
//!         Ok(CallToolResult::success(vec![ContentBlock::text(
//!             format!("Hello, {}!", params.0.name)
//!         )]))
//!     }
//! }
//!
//! #[tool_handler]
//! impl ServerHandler for Greeter {
//!     fn get_info(&self) -> ServerConfig {
//!         ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
//!             .with_server_info(Implementation::new("greeter", "1.0.0"))
//!     }
//! }
//! ```

// Re-export rmcp macros for tool definition
pub use rmcp::{tool, tool_handler, tool_router};

// Re-export core types from rmcp
pub use rmcp::ServerHandler;

// Re-export model types
pub use rmcp::model::{
    // Tool types
    CallToolResult,
    // Content types
    ContentBlock,
    CustomNotification,
    // Custom protocol extensions (rmcp 0.12.0+)
    CustomRequest,
    CustomResult,
    // Server identity and capabilities
    Implementation,
    ServerCapabilities,
    ServerConfig,
    Tool,
};

// Re-export handler types
pub use rmcp::handler::server::tool::ToolRouter;
pub use rmcp::handler::server::wrapper::{Json, Parameters};

// Re-export service types for handler signatures
pub use rmcp::service::{RequestContext, RoleServer};

// Re-export schemars for schema derivation
pub use rmcp::schemars;

// Re-export ErrorData for tool error handling
pub use rmcp::ErrorData as McpError;

// Re-export transport for stdio server
pub use rmcp::transport::io::stdio;

// Re-export service traits for serving
pub use rmcp::ServiceExt;

/// Marker trait for SDK MCP servers
///
/// Any type implementing `rmcp::ServerHandler` automatically implements this trait,
/// making it usable as an SDK MCP server.
pub trait SdkMcpServer: rmcp::ServerHandler {}
impl<T: rmcp::ServerHandler> SdkMcpServer for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    // Guards the re-exported surface: an rmcp upgrade that renames or drops any
    // of these types breaks this test instead of downstream servers.
    #[derive(Deserialize, schemars::JsonSchema)]
    struct AddParams {
        a: f64,
        b: f64,
    }

    #[derive(Clone)]
    struct Calculator {
        tool_router: ToolRouter<Self>,
    }

    #[tool_router]
    impl Calculator {
        fn new() -> Self {
            Self {
                tool_router: Self::tool_router(),
            }
        }

        #[tool(description = "Add two numbers")]
        async fn add(
            &self,
            Parameters(params): Parameters<AddParams>,
        ) -> Result<CallToolResult, McpError> {
            Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "{}",
                params.a + params.b
            ))]))
        }
    }

    // The expansion of `#[tool_handler]` owns these async trait bodies.
    #[allow(clippy::unused_async_trait_impl)]
    #[tool_handler]
    impl ServerHandler for Calculator {
        fn get_info(&self) -> ServerConfig {
            ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
                .with_server_info(Implementation::new("calculator", "1.0.0"))
        }
    }

    fn assert_sdk_server<T: SdkMcpServer>(_: &T) {}

    #[test]
    fn tool_macros_register_routes() {
        let calc = Calculator::new();
        assert_sdk_server(&calc);

        let tools = calc.tool_router.list_all();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "add");
        assert!(calc.tool_router.has_route("add"));
    }

    #[test]
    fn server_config_advertises_tools_and_identity() {
        let info = Calculator::new().get_info();
        assert!(info.capabilities.tools.is_some());
        assert_eq!(info.server_info.name, "calculator");
        assert_eq!(info.server_info.version, "1.0.0");
    }
}
