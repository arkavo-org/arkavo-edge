//! MCP interception proxy.
//!
//! The proxy presents itself as an MCP server on stdio toward a downstream
//! agent/client and forwards to a single upstream MCP server spawned as a
//! subprocess. Every `tools/call` is evaluated against a [`PolicyHook`]
//! before forwarding; denied calls are answered with an MCP error and never
//! reach the upstream server. All other methods (`initialize`, `tools/list`,
//! notifications, ...) are passed through, relaying upstream responses —
//! including errors — verbatim.
//!
//! Calling identity is deliberately out of scope for this slice; the
//! [`CallContext`] struct is the extension point where a principal will be
//! attached without changing the [`PolicyHook`] trait.

pub mod policy;
mod proxy;
mod upstream;

pub use policy::{AllowAllPolicy, CallContext, Decision, DenyListPolicy, PolicyHook};
pub use proxy::{
    INVALID_REQUEST, McpProxy, PARSE_ERROR, POLICY_DENIED, ProxyConfig, ProxyError, UPSTREAM_ERROR,
};
pub use upstream::{UpstreamConnection, UpstreamError};
