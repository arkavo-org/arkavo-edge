//! Policy evaluation hook applied to every `tools/call` before it is
//! forwarded upstream.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;

/// Context for a single `tools/call` policy evaluation.
///
/// Fields are additive by design: a calling-identity/principal field can be
/// introduced later without changing the [`PolicyHook`] trait signature, so
/// existing implementations keep compiling.
#[derive(Debug, Clone)]
pub struct CallContext {
    /// Name of the tool being invoked.
    pub tool_name: String,
    /// Arguments supplied by the caller.
    pub arguments: Value,
    /// Raw CWT permit bytes from `params._meta.arkavo.permit`, if present.
    pub permit: Option<Vec<u8>>,
    /// Raw proof-of-possession signature from `params._meta.arkavo.pop`.
    pub proof: Option<Vec<u8>>,
}

/// Outcome of a policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The call may be forwarded upstream.
    Allow,
    /// The call must be rejected; the reason is returned to the client.
    Deny {
        /// Human-readable explanation included in the MCP error response.
        reason: String,
    },
}

impl Decision {
    /// Whether the decision allows forwarding.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// Hook evaluated on every `tools/call` request before it reaches upstream.
///
/// Implementations must be cheap and non-blocking; they run on the proxy's
/// request path.
#[async_trait]
pub trait PolicyHook: Send + Sync {
    /// Decide whether the call described by `ctx` may proceed.
    async fn evaluate(&self, ctx: &CallContext) -> Decision;
}

/// Default policy that permits every tool call.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAllPolicy;

#[async_trait]
impl PolicyHook for AllowAllPolicy {
    async fn evaluate(&self, _ctx: &CallContext) -> Decision {
        Decision::Allow
    }
}

/// Static-rule policy that denies calls to an explicit set of tool names.
#[derive(Debug, Clone, Default)]
pub struct DenyListPolicy {
    denied: HashSet<String>,
}

impl DenyListPolicy {
    /// Create a policy denying each tool name in `tools`.
    pub fn new<I, S>(tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            denied: tools.into_iter().map(Into::into).collect(),
        }
    }
}

#[async_trait]
impl PolicyHook for DenyListPolicy {
    async fn evaluate(&self, ctx: &CallContext) -> Decision {
        if self.denied.contains(&ctx.tool_name) {
            Decision::Deny {
                reason: format!("tool '{}' is on the deny list", ctx.tool_name),
            }
        } else {
            Decision::Allow
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]
    use super::*;
    use serde_json::json;

    fn ctx(tool_name: &str) -> CallContext {
        CallContext {
            tool_name: tool_name.to_string(),
            arguments: json!({}),
            permit: None,
            proof: None,
        }
    }

    #[tokio::test]
    async fn allow_all_permits_any_tool() {
        let policy = AllowAllPolicy;
        assert!(policy.evaluate(&ctx("anything")).await.is_allowed());
    }

    #[tokio::test]
    async fn deny_list_blocks_listed_tool_with_reason() {
        let policy = DenyListPolicy::new(["shell_exec"]);
        match policy.evaluate(&ctx("shell_exec")).await {
            Decision::Deny { reason } => assert!(reason.contains("shell_exec")),
            Decision::Allow => panic!("expected deny decision"),
        }
    }

    #[tokio::test]
    async fn deny_list_permits_unlisted_tool() {
        let policy = DenyListPolicy::new(["shell_exec"]);
        assert!(policy.evaluate(&ctx("read_file")).await.is_allowed());
    }
}
