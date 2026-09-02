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
//! Traffic is one-way by design in this slice: **server-initiated requests
//! are not relayed to the downstream client.** A client's permit and proof
//! authorize the call it made, not whatever the upstream server decides to
//! ask for afterwards, so a `sampling/createMessage`, `elicitation/create`
//! or `roots/list` from upstream is answered with JSON-RPC `-32601` rather
//! than forwarded — the server learns at once instead of blocking until the
//! per-request timeout. Relaying them is future work that needs its own
//! authorization story.
//!
//! Downstream input is bounded: one message may be at most 1 MiB, a
//! `_meta.arkavo` credential at most the encoded size of the largest permit,
//! and the dispatch gate caps the arguments it will hash. A JSON-RPC batch
//! (a top-level array) is refused with `INVALID_REQUEST` rather than
//! silently dropped.
//!
//! Calling identity is deliberately out of scope for this slice; the
//! [`CallContext`] struct is the extension point where a principal will be
//! attached without changing the [`PolicyHook`] trait.

pub mod framing;
mod permit_hook;
pub mod policy;
mod proxy;
mod upstream;

pub use framing::MAX_LINE_BYTES;
pub use permit_hook::PermitPolicy;
pub use policy::{
    AllowAllPolicy, CallContext, Credential, Decision, DenyListPolicy, ForwardOutcome, PolicyHook,
};
pub use proxy::{
    INVALID_REQUEST, McpProxy, PARSE_ERROR, POLICY_DENIED, ProxyConfig, ProxyError, UPSTREAM_ERROR,
};
pub use upstream::{UpstreamConnection, UpstreamError};
