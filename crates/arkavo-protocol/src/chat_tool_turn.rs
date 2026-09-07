//! Replaying one assistant turn and the tool outputs that answer it.
//!
//! Moved out of `chat_session` unchanged: the chat loop appends these whole
//! slices so a turn's calls can never reach history without their results.

use arkavo_llm::{Message, ToolExecutionResult};

/// Every call the assistant turn obliges the next request to answer, as
/// `(call_id, tool_name)`.
///
/// A Responses turn replays its provider state verbatim, so the native
/// `function_call` records — not the parsed calls — decide which outputs the
/// provider demands.
/// Chat Completions turns carry only parsed calls, and a call the local parser
/// pulled out of prose has no id of its own, so one is synthesized the same way
/// the streamed tool-call deltas synthesize theirs.
fn pending_call_ids(response: &arkavo_llm::ProviderResponse) -> Vec<(String, String)> {
    let mut calls: Vec<(String, String)> = response
        .provider_state
        .native_calls()
        .map(|(call_id, name)| (call_id.to_string(), name.to_string()))
        .collect();
    for (idx, call) in response.tool_calls.iter().enumerate() {
        let call_id = call
            .call_id
            .clone()
            .unwrap_or_else(|| format!("call_{idx}"));
        if !calls.iter().any(|(known, _)| *known == call_id) {
            calls.push((call_id, call.tool_name.clone()));
        }
    }
    calls
}

/// Replay one turn's tool results in the role the provider's next request needs.
///
/// Providers that issued native calls reject a continuation that answers them
/// with anything but a paired tool-role message, so each result becomes its own
/// `Role::Tool` message keyed by the call id. Calls parsed out of a Responses
/// turn's prose have no provider-side call to answer and stay a user summary.
fn tool_result_messages(
    response: &arkavo_llm::ProviderResponse,
    results: &[ToolExecutionResult],
) -> Vec<Message> {
    if !response.tool_results_use_tool_role() {
        return vec![Message::user(format_tool_results(results))];
    }
    let pending = pending_call_ids(response);
    results
        .iter()
        .enumerate()
        .map(|(idx, result)| {
            let call_id = result
                .call_id
                .clone()
                .or_else(|| pending.get(idx).map(|(id, _)| id.clone()))
                .unwrap_or_else(|| format!("call_{idx}"));
            Message::tool_result(
                arkavo_llm::tool_result::bounded_tool_output(
                    serde_json::json!({
                        "result": result.result, "success": result.success, "error": result.error
                    })
                    .to_string(),
                ),
                call_id,
                result.tool_name.clone(),
            )
        })
        .collect()
}

/// Outputs for a turn whose calls this session cannot run.
///
/// Dropping them orphans the assistant's `function_call` items and the next
/// request fails with "No tool output found", so the model is told the tool is
/// unavailable instead of being left waiting for a result that never comes.
fn unavailable_tool_results(response: &arkavo_llm::ProviderResponse) -> Vec<ToolExecutionResult> {
    pending_call_ids(response)
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
pub fn executed_tool_turn(
    response: &arkavo_llm::ProviderResponse,
    results: &[ToolExecutionResult],
) -> Vec<Message> {
    let mut messages = vec![response.as_assistant_message()];
    if !results.is_empty() {
        messages.extend(tool_result_messages(response, results));
    }
    messages
}

/// The same turn when no tool registry is attached: the calls cannot run, so
/// each one is answered with an "unavailable" output rather than left orphaned.
pub fn unregistered_tool_turn(response: &arkavo_llm::ProviderResponse) -> Vec<Message> {
    executed_tool_turn(response, &unavailable_tool_results(response))
}

/// Maximum characters per tool result to prevent exceeding LLM token limits
const MAX_TOOL_RESULT_CHARS: usize = 200_000;

/// Format tool execution results for adding to conversation context
fn format_tool_results(results: &[ToolExecutionResult]) -> String {
    use std::fmt::Write;

    let mut formatted = String::from("Tool execution results:\n\n");

    for result in results {
        let _ = writeln!(formatted, "Tool: {}", result.tool_name);
        if result.success {
            let result_json =
                serde_json::to_string_pretty(&result.result).unwrap_or_else(|_| "{}".to_string());

            // Truncate large results to prevent exceeding LLM token limits
            if result_json.len() > MAX_TOOL_RESULT_CHARS {
                let mut end = MAX_TOOL_RESULT_CHARS;
                while !result_json.is_char_boundary(end) {
                    end -= 1;
                }
                let truncated = &result_json[..end];
                let break_point = truncated
                    .rfind('\n')
                    .or_else(|| truncated.rfind(' '))
                    .unwrap_or(end);
                let _ = writeln!(
                    formatted,
                    "Result (truncated from {} to {} chars):\n{}...\n[OUTPUT TRUNCATED]",
                    result_json.len(),
                    break_point,
                    &result_json[..break_point]
                );
            } else {
                let _ = writeln!(formatted, "Result: {result_json}");
            }
        } else {
            let error_msg = result.error.as_deref().unwrap_or("Unknown error");
            let _ = writeln!(formatted, "Error: {error_msg}");
        }
        formatted.push('\n');
    }

    formatted
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

    #[test]
    fn summary_tool_results_truncate_unicode_without_panicking() {
        let mut result = executed("read_file", None);
        result.result = serde_json::json!("界".repeat(200_000));
        let summary = format_tool_results(&[result]);
        assert!(summary.contains("OUTPUT TRUNCATED"));
        assert!(summary.len() < 201_000);
    }

    #[test]
    fn paired_tool_results_are_bounded() {
        let response = arkavo_llm::ProviderResponse {
            tool_calls: vec![arkavo_llm::tool_parser::ParsedToolCall {
                tool_name: "read_file".into(),
                arguments: serde_json::json!({}),
                call_id: Some("call_large".into()),
            }],
            ..Default::default()
        };
        for success in [true, false] {
            let mut result = executed("read_file", Some("call_large"));
            result.result = serde_json::json!("界".repeat(200_000));
            result.success = success;
            result.error = Some("é".repeat(200_000));
            let messages = tool_result_messages(&response, &[result]);
            assert_eq!(messages[0].tool_call_id.as_deref(), Some("call_large"));
            assert!(messages[0].content.len() <= arkavo_llm::tool_result::MAX_TOOL_RESULT_BYTES);
            assert!(messages[0].content.contains("OUTPUT TRUNCATED"));
        }
    }

    #[spec("ASTRA-002")]
    #[test]
    fn chat_completions_results_replay_as_tool_role_with_call_ids() {
        let response = arkavo_llm::ProviderResponse {
            tool_calls: vec![parsed_call("read_file", Some("call_a"))],
            ..Default::default()
        };
        let messages = tool_result_messages(&response, &[executed("read_file", Some("call_a"))]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, arkavo_llm::Role::Tool);
        assert_eq!(messages[0].tool_call_id.as_deref(), Some("call_a"));
        assert_every_call_is_paired(&response.as_assistant_message(), &messages);
    }

    #[spec("ASTRA-002")]
    #[test]
    fn native_function_call_items_replay_as_tool_role_with_call_ids() {
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
        let messages = tool_result_messages(
            &response,
            &[
                executed("read_file", Some("fc_1")),
                executed("list_dir", Some("fc_2")),
            ],
        );
        assert_eq!(messages.len(), 2);
        assert_every_call_is_paired(&response.as_assistant_message(), &messages);
    }

    #[spec("ASTRA-002")]
    #[test]
    fn prose_extracted_results_stay_a_user_message() {
        let response = arkavo_llm::ProviderResponse {
            provider_state: arkavo_llm::ProviderState::openai_responses(vec![
                serde_json::json!({"type": "reasoning", "id": "rs_1"}),
            ]),
            tool_calls: vec![parsed_call("read_file", None)],
            ..Default::default()
        };
        let messages = tool_result_messages(&response, &[executed("read_file", None)]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, arkavo_llm::Role::User);
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
