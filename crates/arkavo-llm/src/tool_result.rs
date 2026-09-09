//! Pairing one turn's tool outputs with the calls they answer, and bounding
//! them before they are retained in conversation history.
//!
//! Every tool loop in the workspace — the chat session, the CLI loop and the
//! server conductor — replays results through these helpers so a call can never
//! reach the next request without an output carrying its id.

use crate::message::Message;
use crate::provider::ProviderResponse;
use crate::tool_executor::ToolExecutionResult;

pub const MAX_TOOL_RESULT_BYTES: usize = 200_000;

/// A tool error long enough to crowd out the result it explains is itself
/// noise; the head of the message must stay small enough to always survive.
const MAX_TOOL_ERROR_BYTES: usize = 8_192;

const MARKER: &str = "\n[OUTPUT TRUNCATED - result too large for LLM context]";

/// The longest prefix of `text` that fits in `max` bytes without splitting a
/// UTF-8 scalar. Model and tool output can end anywhere in a multi-byte
/// character, so byte slicing it panics.
pub fn char_boundary_prefix(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Tool output is untrusted and may exceed the next provider's context window.
/// The marker is included in the byte allowance, with UTF-8 boundaries preserved.
pub(crate) fn bounded_tool_output(mut output: String) -> String {
    if output.len() > MAX_TOOL_RESULT_BYTES {
        let end = char_boundary_prefix(&output, MAX_TOOL_RESULT_BYTES - MARKER.len()).len();
        output.truncate(end);
        output.push_str(MARKER);
    }
    output
}

/// The JSON body of one tool-role message.
///
/// `success` and the truncation flag lead the object because an oversized
/// payload is cut from the tail: a model that only ever reads the head must
/// still learn whether the call succeeded and whether it is seeing all of the
/// output. A truncated body is deliberately not closed — it is no longer valid
/// JSON, and pretending otherwise would hide the cut.
pub(crate) fn bounded_tool_result_json(result: &ToolExecutionResult) -> String {
    let raw_error = result.error.as_deref();
    let error = raw_error.map(|error| char_boundary_prefix(error, MAX_TOOL_ERROR_BYTES));
    // Cutting the error is itself a truncation: the flag must never read false
    // while part of the message is missing.
    let error_cut = raw_error.is_some_and(|error| error.len() > MAX_TOOL_ERROR_BYTES);
    let error_json = serde_json::to_string(&error).unwrap_or_else(|_| "null".to_string());
    let payload = serde_json::to_string(&result.result).unwrap_or_else(|_| "null".to_string());
    let head = |truncated: bool| {
        format!(
            "{{\"success\":{},\"truncated\":{truncated},\"error\":{error_json},\"result\":",
            result.success
        )
    };
    // `<` rather than `<= MAX - 1`: the spare byte is the closing brace.
    let closed_fits = |head: &str| head.len() + payload.len() < MAX_TOOL_RESULT_BYTES;

    let flagged = head(error_cut);
    if closed_fits(&flagged) {
        return format!("{flagged}{payload}}}");
    }
    let head = head(true);
    let room = MAX_TOOL_RESULT_BYTES.saturating_sub(head.len() + MARKER.len());
    let body = format!("{head}{}{MARKER}", char_boundary_prefix(&payload, room));
    // A pathological error message can outgrow the whole budget on its own.
    bounded_tool_output(body)
}

/// One tool output in the role the provider's next request needs.
///
/// Providers that issued native calls reject a continuation that answers them
/// with anything but a paired tool-role message. Calls the provider never
/// recorded have nothing to pair against, so their output is narrated back as
/// user text instead.
pub fn tool_feedback_message(
    content: impl Into<String>,
    call_id: impl Into<String>,
    tool_name: impl Into<String>,
    use_tool_role: bool,
) -> Message {
    let content = bounded_tool_output(content.into());
    let tool_name = tool_name.into();
    if use_tool_role {
        Message::tool_result(content, call_id, tool_name)
    } else {
        Message::user(format!("[Tool result {tool_name}]: {content}"))
    }
}

impl ProviderResponse {
    /// Every call this turn obliges the next request to answer, as
    /// `(call_id, tool_name)`.
    ///
    /// A Responses turn replays its provider state verbatim, so the native
    /// `function_call` records — not the parsed calls — decide which outputs
    /// the provider demands. Chat Completions turns carry only parsed calls,
    /// and a call the local parser pulled out of prose has no id of its own,
    /// so one is synthesized the same way the streamed deltas synthesize theirs.
    pub fn pending_call_ids(&self) -> Vec<(String, String)> {
        let mut calls: Vec<(String, String)> = self
            .provider_state
            .native_calls()
            .map(|(call_id, name)| (call_id.to_string(), name.to_string()))
            .collect();
        for (idx, call) in self.tool_calls.iter().enumerate() {
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

    /// Outputs telling the model that every call this turn issued could not be run.
    ///
    /// Dropping the calls instead orphans them: a Responses continuation replays
    /// the assistant's items verbatim, so the next request carries a
    /// `function_call` with no `function_call_output` and the API rejects it with
    /// "No tool output found". `reason` completes the sentence "Tool 'x' is
    /// unavailable: ...".
    pub fn unanswered_tool_results(&self, reason: &str) -> Vec<ToolExecutionResult> {
        self.pending_call_ids()
            .into_iter()
            .map(|(call_id, tool_name)| {
                let message = format!("Tool '{tool_name}' is unavailable: {reason}");
                ToolExecutionResult {
                    result: serde_json::json!({"error": message}),
                    error: Some(message),
                    tool_name,
                    call_id: Some(call_id),
                    success: false,
                    schema_hint: None,
                }
            })
            .collect()
    }

    /// This turn as history must hold it: the assistant message followed by the
    /// outputs answering its calls, in the order the next request replays them.
    ///
    /// Callers append the whole slice, so a call can never be committed to
    /// history — or persisted to a session — without its answer.
    pub fn recorded_turn(&self, results: &[ToolExecutionResult]) -> Vec<Message> {
        let mut messages = vec![self.as_assistant_message()];
        if !results.is_empty() {
            messages.extend(self.tool_result_messages(results));
        }
        messages
    }

    /// Replay this turn's tool results in the role the next request needs.
    ///
    /// A turn with no call to pair against — a Responses turn whose items hold
    /// no `function_call`, or one whose calls a loop recovered from a markdown
    /// fence in prose — keeps its outputs as a single user summary, because a
    /// tool-role message would arrive with nothing to answer.
    pub fn tool_result_messages(&self, results: &[ToolExecutionResult]) -> Vec<Message> {
        let pending = self.pending_call_ids();
        if pending.is_empty() || !self.tool_results_use_tool_role() {
            return vec![Message::user(
                crate::tool_result_summary::tool_result_summary(results),
            )];
        }
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
                    bounded_tool_result_json(result),
                    call_id,
                    result.tool_name.clone(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Role;
    use crate::provider_state::ProviderState;
    use crate::tool_parser::ParsedToolCall;
    use arkavo_test_macros::spec;

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

    fn parsed_call(name: &str, call_id: Option<&str>) -> ParsedToolCall {
        ParsedToolCall {
            tool_name: name.to_string(),
            arguments: serde_json::json!({}),
            call_id: call_id.map(str::to_string),
        }
    }

    fn function_call_item(call_id: &str, name: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "function_call", "call_id": call_id, "name": name, "arguments": "{}"
        })
    }

    #[test]
    fn bounds_unicode_output_including_marker() {
        let output = bounded_tool_output("界".repeat(MAX_TOOL_RESULT_BYTES));
        assert!(output.len() <= MAX_TOOL_RESULT_BYTES);
        assert!(output.ends_with("[OUTPUT TRUNCATED - result too large for LLM context]"));
        assert!(output.starts_with("界"));
    }

    #[test]
    fn preserves_small_output() {
        assert_eq!(bounded_tool_output("{\"ok\":true}".into()), "{\"ok\":true}");
    }

    #[test]
    fn char_boundary_prefix_never_splits_a_scalar() {
        assert_eq!(char_boundary_prefix("界界", 4), "界");
        assert_eq!(char_boundary_prefix("界界", 2), "");
        assert_eq!(char_boundary_prefix("abc", 10), "abc");
    }

    /// The bug this helper exists for: the old body serialized
    /// `{"error","result","success"}` and cut the tail, so an oversized result
    /// silently dropped the one field telling the model the call had failed.
    #[spec("ASTRA-002")]
    #[test]
    fn truncated_result_keeps_the_success_flag_at_the_head() {
        let mut result = executed("read_file", Some("call_large"));
        result.result = serde_json::json!("界".repeat(MAX_TOOL_RESULT_BYTES));
        result.success = false;
        result.error = Some("é".repeat(MAX_TOOL_RESULT_BYTES));

        let body = bounded_tool_result_json(&result);
        assert!(
            body.starts_with(r#"{"success":false,"truncated":true"#),
            "{}",
            char_boundary_prefix(&body, 64)
        );
        assert!(body.len() <= MAX_TOOL_RESULT_BYTES);
        assert!(body.ends_with(MARKER));
    }

    #[spec("ASTRA-002")]
    #[test]
    fn intact_result_is_valid_json_reporting_success_first() {
        let body = bounded_tool_result_json(&executed("read_file", None));
        assert!(body.starts_with(r#"{"success":true,"truncated":false"#));
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("intact body is JSON");
        assert_eq!(parsed["result"], serde_json::json!({"ok": true}));
        assert_eq!(parsed["error"], serde_json::Value::Null);
    }

    #[spec("ASTRA-002")]
    #[test]
    fn paired_results_are_bounded_for_both_outcomes() {
        let response = ProviderResponse {
            tool_calls: vec![parsed_call("read_file", Some("call_large"))],
            ..Default::default()
        };
        for success in [true, false] {
            let mut result = executed("read_file", Some("call_large"));
            result.result = serde_json::json!("界".repeat(MAX_TOOL_RESULT_BYTES));
            result.success = success;
            let messages = response.tool_result_messages(&[result]);
            assert_eq!(messages[0].tool_call_id.as_deref(), Some("call_large"));
            assert!(messages[0].content.len() <= MAX_TOOL_RESULT_BYTES);
            assert!(
                messages[0]
                    .content
                    .starts_with(&format!("{{\"success\":{success},\"truncated\":true"))
            );
        }
    }

    /// Every native call the assistant issued must be answered by a message
    /// carrying its call id, or the provider rejects the next turn.
    fn assert_every_call_is_paired(turn: &[Message]) {
        let (assistant, followers) = turn.split_first().expect("turn records the assistant");
        assert_eq!(assistant.role, Role::Assistant);
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
                    message.role == Role::Tool && message.tool_call_id.as_deref() == Some(&id)
                }),
                "call {id} has no paired tool result"
            );
        }
    }

    /// A session with no way to run the calls must still answer them: dropping
    /// them orphans the provider's `function_call` items.
    #[spec("ASTRA-002")]
    #[test]
    fn unanswered_calls_are_each_reported_unavailable() {
        let response = ProviderResponse {
            provider_state: ProviderState::openai_responses(vec![
                function_call_item("fc_1", "read_file"),
                function_call_item("fc_2", "list_dir"),
            ]),
            tool_calls: vec![
                parsed_call("read_file", Some("fc_1")),
                parsed_call("list_dir", Some("fc_2")),
            ],
            ..Default::default()
        };
        let results = response.unanswered_tool_results("this session has no tool registry");
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| !result.success));

        let turn = response.recorded_turn(&results);
        assert_every_call_is_paired(&turn);
        assert!(turn[1..].iter().all(|m| m.content.contains("unavailable")));
    }

    #[spec("ASTRA-002")]
    #[test]
    fn an_executed_turn_pairs_results_after_the_assistant_message() {
        let response = ProviderResponse {
            provider_state: ProviderState::openai_responses(vec![function_call_item(
                "fc_hint",
                "read_file",
            )]),
            tool_calls: vec![parsed_call("read_file", Some("fc_hint"))],
            ..Default::default()
        };
        let turn = response.recorded_turn(&[executed("read_file", Some("fc_hint"))]);
        assert_every_call_is_paired(&turn);
    }

    /// A turn that called nothing is recorded alone, with nothing after it.
    #[spec("ASTRA-002")]
    #[test]
    fn a_turn_without_tool_calls_is_recorded_alone() {
        let response = ProviderResponse {
            content: "no tools needed".to_string(),
            ..Default::default()
        };
        assert!(response.unanswered_tool_results("unused").is_empty());
        let turn = response.recorded_turn(&[]);
        assert_eq!(turn.len(), 1);
        assert_eq!(turn[0].role, Role::Assistant);
        assert_eq!(turn[0].content, "no tools needed");
    }

    /// An error long enough to be cut is itself a truncation; a reader must not
    /// see `truncated:false` while part of the message is missing.
    #[spec("ASTRA-002")]
    #[test]
    fn a_cut_error_is_flagged_as_truncated() {
        let mut result = executed("read_file", None);
        result.success = false;
        result.error = Some("é".repeat(MAX_TOOL_ERROR_BYTES));
        let body = bounded_tool_result_json(&result);
        assert!(body.starts_with(r#"{"success":false,"truncated":true"#));
        // The payload still fits, so the object is closed and parseable.
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("closed body is JSON");
        assert_eq!(parsed["truncated"], serde_json::json!(true));
        assert!(parsed["error"].as_str().unwrap().len() <= MAX_TOOL_ERROR_BYTES);
    }

    #[test]
    fn an_error_that_fits_is_not_flagged_as_truncated() {
        let mut result = executed("read_file", None);
        result.success = false;
        result.error = Some("é".repeat(16));
        let body = bounded_tool_result_json(&result);
        assert!(body.starts_with(r#"{"success":false,"truncated":false"#));
        assert!(serde_json::from_str::<serde_json::Value>(&body).is_ok());
    }

    #[spec("ASTRA-002")]
    #[test]
    fn chat_completions_results_replay_as_tool_role_with_call_ids() {
        let response = ProviderResponse {
            tool_calls: vec![parsed_call("read_file", Some("call_a"))],
            ..Default::default()
        };
        let messages = response.tool_result_messages(&[executed("read_file", Some("call_a"))]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::Tool);
        assert_eq!(messages[0].tool_call_id.as_deref(), Some("call_a"));
    }

    #[spec("ASTRA-002")]
    #[test]
    fn every_native_function_call_gets_a_paired_result() {
        let response = ProviderResponse {
            provider_state: ProviderState::openai_responses(vec![
                function_call_item("fc_1", "read_file"),
                function_call_item("fc_2", "list_dir"),
            ]),
            tool_calls: vec![
                parsed_call("read_file", Some("fc_1")),
                parsed_call("list_dir", Some("fc_2")),
            ],
            ..Default::default()
        };
        let messages = response.tool_result_messages(&[
            executed("read_file", Some("fc_1")),
            executed("list_dir", Some("fc_2")),
        ]);
        for call_id in response.provider_state.native_call_ids() {
            assert!(
                messages
                    .iter()
                    .any(|m| m.role == Role::Tool && m.tool_call_id.as_deref() == Some(call_id)),
                "no paired result for {call_id}"
            );
        }
    }

    /// A native call whose result came back without an id is still answered
    /// with the id the provider is waiting on, not a synthetic one.
    #[spec("ASTRA-002")]
    #[test]
    fn results_without_ids_fall_back_to_the_native_call_ids() {
        let response = ProviderResponse {
            provider_state: ProviderState::openai_responses(vec![function_call_item(
                "fc_native",
                "read_file",
            )]),
            tool_calls: vec![parsed_call("read_file", Some("fc_native"))],
            ..Default::default()
        };
        let messages = response.tool_result_messages(&[executed("read_file", None)]);
        assert_eq!(messages[0].tool_call_id.as_deref(), Some("fc_native"));
    }

    /// A loop that recovered calls from a markdown fence pushes an assistant
    /// message with no `tool_calls`; a tool-role result would be orphaned.
    #[spec("ASTRA-002")]
    #[test]
    fn markdown_parsed_calls_stay_a_user_message() {
        let response = ProviderResponse {
            content: "```get_time```".to_string(),
            ..Default::default()
        };
        assert!(response.as_assistant_message().tool_calls.is_empty());
        let messages = response.tool_result_messages(&[executed("get_time", None)]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::User);
    }

    #[spec("ASTRA-002")]
    #[test]
    fn prose_extracted_results_stay_a_user_message() {
        let response = ProviderResponse {
            provider_state: ProviderState::openai_responses(vec![
                serde_json::json!({"type": "reasoning", "id": "rs_1"}),
            ]),
            tool_calls: vec![parsed_call("read_file", None)],
            ..Default::default()
        };
        let messages = response.tool_result_messages(&[executed("read_file", None)]);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, Role::User);
    }

    #[spec("ASTRA-002")]
    #[test]
    fn per_call_feedback_switches_role_on_the_pairing_rule() {
        let text_extracted = ProviderResponse {
            provider_state: ProviderState::openai_responses(vec![
                serde_json::json!({"type": "message", "content": []}),
            ]),
            tool_calls: vec![parsed_call("read", None)],
            ..Default::default()
        };
        let feedback = tool_feedback_message(
            "result",
            "synthetic",
            "read",
            text_extracted.tool_results_use_tool_role(),
        );
        assert_eq!(feedback.role, Role::User);
        assert!(feedback.tool_call_id.is_none());

        let native = ProviderResponse {
            provider_state: ProviderState::openai_responses(vec![function_call_item(
                "native", "read",
            )]),
            ..Default::default()
        };
        let feedback = tool_feedback_message(
            "result",
            "native",
            "read",
            native.tool_results_use_tool_role(),
        );
        assert_eq!(feedback.role, Role::Tool);
        assert_eq!(feedback.tool_call_id.as_deref(), Some("native"));
    }

    #[test]
    fn per_call_feedback_is_bounded() {
        let feedback = tool_feedback_message("界".repeat(MAX_TOOL_RESULT_BYTES), "c", "read", true);
        assert!(feedback.content.len() <= MAX_TOOL_RESULT_BYTES);
        assert!(feedback.content.ends_with(MARKER));
    }
}
