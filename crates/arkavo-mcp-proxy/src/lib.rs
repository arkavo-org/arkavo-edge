//! MCP interception proxy.
//!
//! The proxy presents itself as an MCP server on stdio toward a downstream
//! agent/client and forwards to a single upstream MCP server spawned as a
//! subprocess. Every `tools/call` — whether it arrives as a request or as a
//! notification — is either evaluated against a [`PolicyHook`] or dropped
//! before it can reach the upstream server: a `tools/call` request is
//! evaluated and, if denied, answered with an MCP error instead of being
//! forwarded; a `tools/call` sent as a notification (no `id`, so no
//! response could ever carry a denial back) cannot be policy-evaluated and
//! is dropped outright rather than forwarded. All other methods
//! (`initialize`, `tools/list`, other notifications, ...) are passed
//! through, relaying upstream responses — including errors — verbatim.
//!
//! An allowed `tools/call` still has its `params._meta.arkavo` key removed
//! before forwarding, so the permit and proof-of-possession travelling in
//! it never reach the upstream server; every other `_meta` key is
//! forwarded unchanged.
//!
//! Calling identity is deliberately out of scope for this slice; the
//! [`CallContext`] struct is the extension point where a principal will be
//! attached without changing the [`PolicyHook`] trait.

mod permit_hook;
pub mod policy;
mod proxy;
mod upstream;

pub use permit_hook::PermitPolicy;
pub use policy::{AllowAllPolicy, CallContext, Decision, DenyListPolicy, PolicyHook};
pub use proxy::{
    INVALID_REQUEST, McpProxy, PARSE_ERROR, POLICY_DENIED, ProxyConfig, ProxyError, UPSTREAM_ERROR,
};
pub use upstream::{UpstreamConnection, UpstreamError};
