//! Replaying one assistant turn and the tool outputs that answer it.
//!
//! The chat loop appends these whole slices so a turn's calls can never reach
//! history without their results. Pairing and bounding themselves belong to
//! `arkavo_llm::tool_result`, which every tool loop in the workspace shares.

use arkavo_llm::{Message, ToolExecutionResult};

/// Outputs for a turn whose calls this session cannot run.
///
/// Dropping them orphans the assistant's `function_call` items and the next
/// request fails with "No tool output found", so the model is told the tool is
/// unavailable instead of being left waiting for a result that never comes.
fn unavailable_tool_results(response: &arkavo_llm::ProviderResponse) -> Vec<ToolExecutionResult> {
    response
        .pending_call_ids()
        .into_iter()
        .map(|(call_id, tool_name)| ToolExecutionResult {
            result: serde_json::json!({
                "error": format!("Tool '{tool_name}' is unavailable: this session has no tool registry")
            }),
            error: Some(format!(
                "Tool '{tool_name}' is unavailable: this session has no tool registry"
            )),
            tool_name,
            call_id: Some(call_id),
            success: false,
            schema_hint: None,
        })
        .collect()
}

/// One assistant turn and the outputs answering it.
///
/// In the order the next request must replay them. Every caller appends this
/// whole slice so a turn's calls can never be committed to history without
/// their results.
pub(super) fn executed_tool_turn(
    response: &arkavo_llm::ProviderResponse,
    results: &[ToolExecutionResult],
) -> Vec<Message> {
    let mut messages = vec![response.as_assistant_message()];
    if !results.is_empty() {
        messages.extend(response.tool_result_messages(results));
    }
    messages
}

/// The same turn when no tool registry is attached: the calls cannot run, so
/// each one is answered with an "unavailable" output rather than left orphaned.
pub(super) fn unregistered_tool_turn(response: &arkavo_llm::ProviderResponse) -> Vec<Message> {
    executed_tool_turn(response, &unavailable_tool_results(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn function_call_item(call_id: &str, name: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "function_call", "call_id": call_id, "name": name, "arguments": "{}"
        })
    }

    fn parsed_call(name: &str, call_id: Option<&str>) -> arkavo_llm::ParsedToolCall {
        arkavo_llm::ParsedToolCall {
            tool_name: name.to_string(),
            arguments: serde_json::json!({}),
            call_id: call_id.map(str::to_string),
        }
    }

    fn executed(name: &str, call_id: Option<&str>) -> ToolExecutionResult {
        ToolExecutionResult {
            tool_name: name.to_string(),
            call_id: call_id.map(str::to_string),
            result: serde_json::json!({"ok": true}),
            success: true,
            error: None,
            schema_hint: None,
        }
    }

    /// Every native call the assistant issued must be answered by a message
    /// carrying its call id, or the provider rejects the next turn.
    fn assert_every_call_is_paired(assistant: &Message, followers: &[Message]) {
        let mut ids: Vec<String> = assistant
            .provider_state
            .native_call_ids()
            .map(str::to_string)
            .collect();
        ids.extend(
            assistant
                .tool_calls
                .iter()
                .filter_map(|call| call.id.clone()),
        );
        assert!(!ids.is_empty(), "test fixture must issue at least one call");
        for id in ids {
            assert!(
                followers.iter().any(|message| {
                    message.role == arkavo_llm::Role::Tool
                        && message.tool_call_id.as_deref() == Some(id.as_str())
                }),
                "call {id} has no paired tool result"
            );
        }
    }

    #[spec("ASTRA-002")]
    #[test]
    fn no_tool_registry_branch_still_answers_every_native_call() {
        let response = arkavo_llm::ProviderResponse {
            provider_state: arkavo_llm::ProviderState::openai_responses(vec![
                function_call_item("fc_1", "read_file"),
                function_call_item("fc_2", "list_dir"),
            ]),
            tool_calls: vec![
                parsed_call("read_file", Some("fc_1")),
                parsed_call("list_dir", Some("fc_2")),
            ],
            ..Default::default()
        };
        // Exactly what the no-registry branch appends to the context.
        let context = unregistered_tool_turn(&response);

        let (assistant, followers) = context.split_first().unwrap();
        assert_eq!(assistant.role, arkavo_llm::Role::Assistant);
        assert_every_call_is_paired(assistant, followers);
        assert!(followers.iter().all(|m| m.content.contains("unavailable")));
    }

    #[spec("ASTRA-002")]
    #[test]
    fn hint_retry_branch_pushes_results_after_the_assistant_turn() {
        let response = arkavo_llm::ProviderResponse {
            provider_state: arkavo_llm::ProviderState::openai_responses(vec![function_call_item(
                "fc_hint",
                "read_file",
            )]),
            tool_calls: vec![parsed_call("read_file", Some("fc_hint"))],
            ..Default::default()
        };
        // Exactly what the hint-retry branch appends to the context.
        let results = vec![executed("read_file", Some("fc_hint"))];
        let context = executed_tool_turn(&response, &results);

        let (assistant, followers) = context.split_first().unwrap();
        assert_eq!(assistant.role, arkavo_llm::Role::Assistant);
        assert_every_call_is_paired(assistant, followers);
    }

    /// The hint-retry branch also runs for answers that called no tools; it must
    /// still record the assistant turn and add nothing after it.
    #[spec("ASTRA-002")]
    #[test]
    fn a_turn_without_tool_calls_is_recorded_alone() {
        let response = arkavo_llm::ProviderResponse {
            content: "no tools needed".to_string(),
            ..Default::default()
        };
        let context = executed_tool_turn(&response, &[]);
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].role, arkavo_llm::Role::Assistant);
        assert_eq!(context[0].content, "no tools needed");
    }
}
