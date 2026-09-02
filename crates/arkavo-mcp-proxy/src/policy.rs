//! Policy evaluation hook applied to every `tools/call` before it is
//! forwarded upstream.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;

/// One of the credentials a call carries under `params._meta.arkavo`, as it
/// arrived.
///
/// A hook that only saw `Option<Vec<u8>>` could not tell a client that sent
/// nothing from one that sent something it could not decode, and told both
/// the same unhelpful thing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Credential {
    /// The field was not present.
    #[default]
    Absent,
    /// The field was present but is not base64url without padding.
    Undecodable,
    /// The field was present but longer than any permit or proof can be, so
    /// it was refused without decoding it.
    Oversized,
    /// The decoded bytes.
    Present(Vec<u8>),
}

impl Credential {
    /// The decoded bytes, if this credential has any.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Present(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// Whether the client sent this field at all, in any state.
    pub fn is_present(&self) -> bool {
        !matches!(self, Self::Absent)
    }
}

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
    /// The CWT permit from `params._meta.arkavo.permit`.
    pub permit: Credential,
    /// The proof-of-possession signature from `params._meta.arkavo.pop`.
    pub proof: Credential,
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

    /// Called when a call this hook allowed never reached the upstream
    /// server — the connection failed, or the request timed out — so a hook
    /// that spent something admitting it can give that back.
    ///
    /// It is *not* called when the upstream ran the call and answered with a
    /// JSON-RPC error: that is a completed call whose result happens to be a
    /// failure, and it keeps whatever it spent.
    ///
    /// Default: nothing to return.
    async fn on_forward_failed(&self, _ctx: &CallContext) {}
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
// The `#[tokio::test]` macro expands to `Runtime::block_on`, which
// `.clippy.toml` disallows outside test code.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(tool_name: &str) -> CallContext {
        CallContext {
            tool_name: tool_name.to_string(),
            arguments: json!({}),
            permit: Credential::Absent,
            proof: Credential::Absent,
        }
    }

    #[test]
    fn a_credential_yields_bytes_only_when_it_has_them() {
        assert_eq!(Credential::Present(vec![1, 2]).bytes(), Some(&[1u8, 2][..]));
        assert_eq!(Credential::Absent.bytes(), None);
        assert_eq!(Credential::Undecodable.bytes(), None);
        assert_eq!(Credential::Oversized.bytes(), None);

        // "Present" in the sense the deny messages use: the client sent the
        // field, whatever state it arrived in.
        assert!(!Credential::Absent.is_present());
        assert!(Credential::Undecodable.is_present());
        assert!(Credential::Oversized.is_present());
        assert!(Credential::Present(Vec::new()).is_present());
    }

    /// The default hook does nothing on a failed forward, so an
    /// implementation that spends nothing needs no code for it.
    #[tokio::test]
    async fn on_forward_failed_defaults_to_doing_nothing() {
        AllowAllPolicy.on_forward_failed(&ctx("anything")).await;
        assert!(AllowAllPolicy.evaluate(&ctx("anything")).await.is_allowed());
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
