use std::collections::HashMap;
use std::sync::Arc;

use anthropic_agent_sdk::callbacks::{FnHookCallback, FnPermissionCallback};
use anthropic_agent_sdk::types::{
    HookEvent, HookMatcher, HookOutput, PermissionResult, PermissionResultAllow,
    PermissionResultDeny, ToolPermissionContext,
};
use anthropic_agent_sdk::{CanUseToolCallback, HookMatcherBuilder};
use tracing::debug;

use crate::event_mapper::EventMapper;
use crate::policy_bridge::PolicyBridge;

/// Build SDK hooks that emit AG-UI events via `EventMapper`.
///
/// Registers wildcard hooks for `PostToolUse` and `PostToolUseFailure`
/// so every tool invocation is captured and forwarded as events.
pub fn build_hooks(event_mapper: Arc<EventMapper>) -> HashMap<HookEvent, Vec<HookMatcher>> {
    let mut hooks = HashMap::new();

    // PostToolUse: emit tool result events for successful tool calls
    let mapper_success = event_mapper.clone();
    let success_hook = Arc::new(FnHookCallback::new(move |input, tool_use_id, _ctx| {
        let mapper = mapper_success.clone();
        let tool_id = tool_use_id.unwrap_or_default();
        Box::pin(async move {
            let tool_name = input["tool_name"].as_str().unwrap_or(&tool_id);
            debug!("PostToolUse hook: {tool_name}");
            mapper
                .emit_tool_result_typed(
                    "active",
                    tool_name,
                    true,
                    &input,
                    input["duration_ms"].as_u64().unwrap_or(0),
                )
                .await;
            Ok(HookOutput::default())
        })
    }));

    let success_matcher = HookMatcherBuilder::new(Some("*"))
        .add_hook(success_hook)
        .build();
    hooks.insert(HookEvent::PostToolUse, vec![success_matcher]);

    // PostToolUseFailure: emit failure events
    let mapper_fail = event_mapper;
    let fail_hook = Arc::new(FnHookCallback::new(move |input, tool_use_id, _ctx| {
        let mapper = mapper_fail.clone();
        let tool_id = tool_use_id.unwrap_or_default();
        Box::pin(async move {
            let tool_name = input["tool_name"].as_str().unwrap_or(&tool_id);
            let error_msg = input["error"].as_str().unwrap_or("unknown error");
            debug!("PostToolUseFailure hook: {tool_name}: {error_msg}");
            mapper
                .emit_tool_result_typed(
                    "active",
                    tool_name,
                    false,
                    &serde_json::json!({"error": error_msg}),
                    0,
                )
                .await;
            Ok(HookOutput::default())
        })
    }));

    let fail_matcher = HookMatcherBuilder::new(Some("*"))
        .add_hook(fail_hook)
        .build();
    hooks.insert(HookEvent::PostToolUseFailure, vec![fail_matcher]);

    hooks
}

/// Build an SDK permission callback that delegates to `PolicyBridge`.
///
/// Maps Claude Code tool names to the policy bridge's permission categories
/// (read, write, exec, web_search) and enforces file path policies.
pub fn build_permission_callback(
    policy_bridge: Arc<PolicyBridge>,
    event_mapper: Arc<EventMapper>,
) -> CanUseToolCallback {
    Arc::new(FnPermissionCallback::new(
        move |tool_name: String, input: serde_json::Value, _ctx: ToolPermissionContext| {
            let bridge = policy_bridge.clone();
            let mapper = event_mapper.clone();
            Box::pin(async move {
                let request = serde_json::json!({
                    "tool": map_sdk_tool_name(&tool_name),
                    "path": input.get("file_path")
                        .or_else(|| input.get("path"))
                        .and_then(|p| p.as_str()),
                });

                let allowed = bridge.check_tool_permission(request).await.unwrap_or(false);

                let reason = if allowed {
                    "permitted by policy"
                } else {
                    "denied by policy"
                };
                mapper
                    .emit_permission_decision("active", &tool_name, allowed, reason)
                    .await;

                if allowed {
                    Ok(PermissionResult::Allow(PermissionResultAllow {
                        updated_input: None,
                        updated_permissions: None,
                    }))
                } else {
                    Ok(PermissionResult::Deny(PermissionResultDeny {
                        message: format!("Tool '{tool_name}' denied by Arkavo policy"),
                        interrupt: false,
                    }))
                }
            })
        },
    ))
}

/// Map Claude Code SDK tool names to our policy bridge categories
fn map_sdk_tool_name(sdk_name: &str) -> &str {
    match sdk_name {
        "Read" | "Glob" | "Grep" | "LS" => "read_file",
        "Write" | "Edit" | "NotebookEdit" => "write_file",
        "Bash" => "execute_command",
        "WebFetch" | "WebSearch" => "web_search",
        _ => sdk_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_sdk_tool_name() {
        assert_eq!(map_sdk_tool_name("Read"), "read_file");
        assert_eq!(map_sdk_tool_name("Glob"), "read_file");
        assert_eq!(map_sdk_tool_name("Write"), "write_file");
        assert_eq!(map_sdk_tool_name("Bash"), "execute_command");
        assert_eq!(map_sdk_tool_name("WebFetch"), "web_search");
        assert_eq!(map_sdk_tool_name("CustomTool"), "CustomTool");
    }

    #[tokio::test]
    async fn test_build_hooks_has_expected_events() {
        let writer = Arc::new(arkavo_events::EventWriter::new(
            arkavo_events::EventWriterConfig::default(),
        ));
        let mapper = Arc::new(EventMapper::new("test-agent".to_string(), writer));
        let hooks = build_hooks(mapper);

        assert!(hooks.contains_key(&HookEvent::PostToolUse));
        assert!(hooks.contains_key(&HookEvent::PostToolUseFailure));
        assert_eq!(hooks.len(), 2);
    }
}
