//! Anthropic Messages content blocks, and the conversion into them.
//!
//! Anthropic accepts only the `user` and `assistant` roles; a tool call and its
//! result are content blocks inside those turns. Rendering them natively is
//! what keeps an assistant's calls — name, arguments and id — in the transcript
//! the model reads back, and lets the answering turn point at the call it
//! answers instead of describing it in prose.
//!
//! Thinking blocks are not replayed: this crate carries a turn's reasoning as
//! text with no signature to return, and a history without thinking blocks is
//! a shape the API accepts (the model simply loses that reasoning).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Message, Role};

/// One block of a Messages `content` array.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        /// A tool that returned nothing sends no content at all: the field is
        /// optional on the wire, and an empty one risks the same rejection an
        /// empty text block gets.
        #[serde(skip_serializing_if = "String::is_empty")]
        content: String,
    },
}

impl ContentBlock {
    fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    fn is_tool_result(&self) -> bool {
        matches!(self, Self::ToolResult { .. })
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ApiMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

impl ApiMessage {
    fn new(role: &str, content: Vec<ContentBlock>) -> Self {
        Self {
            role: role.to_string(),
            content,
        }
    }

    pub(super) fn user_text(text: impl Into<String>) -> Self {
        Self::new("user", vec![ContentBlock::text(text)])
    }

    /// Every text block's text, for callers that only need the prose.
    #[cfg(test)]
    pub(super) fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}

/// One block of an assistant's response.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(super) enum ResponseContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    /// A block type this build does not render. Ignoring it keeps a future
    /// block from failing a turn whose text and calls are readable.
    #[serde(other)]
    Other,
}

/// The parts of a Messages response this crate reads; serde ignores the rest.
#[derive(Debug, Deserialize)]
pub(super) struct MessageResponse {
    pub content: Vec<ResponseContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: Usage,
}

#[derive(Debug, Deserialize)]
pub(super) struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Convert this crate's messages into Anthropic turns.
///
/// The system prompt travels in its own request field, so it is returned
/// separately; multiple system messages concatenate.
pub(super) fn convert_messages(messages: Vec<Message>) -> (Option<String>, Vec<ApiMessage>) {
    let native = native_call_ids(&messages);
    let mut system: Option<String> = None;
    let mut turns: Vec<ApiMessage> = Vec::new();

    for message in messages {
        let content = message.content.trim().to_string();
        let (role, blocks) = match message.role {
            Role::System => {
                if !content.is_empty() {
                    system = Some(match system {
                        Some(existing) => format!("{existing}\n\n{content}"),
                        None => content,
                    });
                }
                continue;
            }
            Role::User => ("user", text_block(content)),
            Role::Assistant => ("assistant", assistant_blocks(content, &message, &native)),
            // A tool result answers a call by id when that call was sent as a
            // `tool_use` block. Otherwise it must still reach the model, and it
            // must not arrive as an assistant turn: Anthropic continues a
            // trailing assistant message as prefill, so the model would finish
            // its own tool output instead of answering it.
            Role::Tool => (
                "user",
                match message.tool_call_id.as_deref() {
                    Some(id) if native.contains(id) => vec![ContentBlock::ToolResult {
                        tool_use_id: id.to_string(),
                        content: message.content.clone(),
                    }],
                    _ => vec![ContentBlock::text(message.tool_result_as_user_text())],
                },
            ),
        };
        if blocks.is_empty() {
            continue;
        }
        push_turn(&mut turns, role, blocks);
    }

    // Anthropic requires the conversation to open on a user turn.
    if turns.first().is_none_or(|turn| turn.role != "user") {
        turns.insert(0, ApiMessage::user_text("Hello"));
    }
    (system, turns)
}

/// An empty text block is rejected, so an empty turn carries no blocks at all.
fn text_block(content: String) -> Vec<ContentBlock> {
    if content.is_empty() {
        Vec::new()
    } else {
        vec![ContentBlock::text(content)]
    }
}

/// Blocks for one assistant turn: its text, then the calls it issued.
///
/// A call the transcript cannot pair — no id, or no result answering it — is
/// rendered as text with its arguments instead of a `tool_use` block. An
/// unanswered `tool_use` is rejected by the API, and dropping the call would
/// hide from the model what it had already asked for.
fn assistant_blocks(
    content: String,
    message: &Message,
    native: &HashSet<String>,
) -> Vec<ContentBlock> {
    let mut blocks = text_block(content);
    for call in &message.tool_calls {
        match call
            .id
            .as_deref()
            .filter(|id| native.contains(*id))
            .zip(call_input(&call.arguments))
        {
            Some((id, input)) => blocks.push(ContentBlock::ToolUse {
                id: id.to_string(),
                name: call.name.clone(),
                input,
            }),
            None => blocks.push(ContentBlock::text(format!(
                "[Tool call {}]: {}",
                call.name, call.arguments
            ))),
        }
    }
    blocks
}

/// A call's arguments as the object the API expects.
///
/// A call that took no arguments carries an empty string in this crate; a value
/// that is not an object at all cannot be a `tool_use` input, and the caller
/// renders that call as text rather than sending arguments the model never gave.
fn call_input(arguments: &str) -> Option<Value> {
    if arguments.trim().is_empty() {
        return Some(Value::Object(serde_json::Map::new()));
    }
    serde_json::from_str(arguments)
        .ok()
        .filter(Value::is_object)
}

/// Call ids that can travel as `tool_use`/`tool_result` pairs.
///
/// A pair is only safe when every one of the assistant's calls can be rendered
/// as a `tool_use` block — an id, arguments that are an object — and the run of
/// tool results that immediately follows answers all of them: Anthropic rejects
/// a `tool_use` without its result and a `tool_result` without its call, so both
/// halves of a round have to fall back to text together.
fn native_call_ids(messages: &[Message]) -> HashSet<String> {
    let mut native = HashSet::new();
    for (index, message) in messages.iter().enumerate() {
        if message.role != Role::Assistant || message.tool_calls.is_empty() {
            continue;
        }
        let answered: HashSet<&str> = messages[index + 1..]
            .iter()
            .take_while(|next| next.role == Role::Tool)
            .filter_map(|next| next.tool_call_id.as_deref())
            .collect();
        let ids: Option<Vec<&str>> = message
            .tool_calls
            .iter()
            .map(|call| {
                call.id
                    .as_deref()
                    .filter(|id| !id.is_empty() && answered.contains(id))
                    .filter(|_| call_input(&call.arguments).is_some())
            })
            .collect();
        if let Some(ids) = ids {
            native.extend(ids.into_iter().map(str::to_string));
        }
    }
    native
}

/// Append a turn, merging into the previous one when the role repeats.
///
/// Anthropic requires alternating roles. Everything keeps the order it was
/// said in, except a merged turn's `tool_result` blocks, which move ahead of
/// its prose: the API reads a user message's results before its text.
fn push_turn(turns: &mut Vec<ApiMessage>, role: &str, blocks: Vec<ContentBlock>) {
    match turns.last_mut() {
        Some(last) if last.role == role => {
            for block in blocks {
                if block.is_tool_result() {
                    let at = last
                        .content
                        .iter()
                        .position(|block| !block.is_tool_result())
                        .unwrap_or(last.content.len());
                    last.content.insert(at, block);
                } else {
                    last.content.push(block);
                }
            }
        }
        _ => turns.push(ApiMessage::new(role, blocks)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolCall;
    use arkavo_test_macros::spec;
    use serde_json::json;

    fn call(name: &str, arguments: &str, id: Option<&str>) -> ToolCall {
        ToolCall {
            name: name.to_string(),
            arguments: arguments.to_string(),
            id: id.map(str::to_string),
        }
    }

    fn blocks(turn: &ApiMessage) -> Value {
        serde_json::to_value(&turn.content).unwrap()
    }

    #[test]
    fn system_prompts_travel_separately_and_roles_alternate() {
        let (system, turns) = convert_messages(vec![
            Message::system("You are a helpful assistant"),
            Message::user("Hello"),
            Message::assistant("Hi there!"),
            Message::user("How are you?"),
        ]);
        assert_eq!(system.as_deref(), Some("You are a helpful assistant"));
        assert_eq!(turns.len(), 3);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[1].role, "assistant");
        assert_eq!(turns[2].role, "user");
        assert_eq!(turns[1].text(), "Hi there!");
    }

    #[test]
    fn repeated_roles_merge_into_one_turn() {
        let (_, turns) = convert_messages(vec![
            Message::user("First message"),
            Message::user("Second message"),
            Message::assistant("Response"),
        ]);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].text(), "First message\n\nSecond message");
        assert_eq!(turns[1].role, "assistant");
    }

    /// The calling turn often carries no text at all. Dropping it loses the
    /// call the model made, and with it the arguments the result answers.
    #[spec("ASTRA-002")]
    #[test]
    fn a_silent_tool_calling_turn_is_preserved_with_its_arguments() {
        let (_, turns) = convert_messages(vec![
            Message::user("what is the weather in Dublin"),
            Message::assistant_with_tool_calls(
                "",
                vec![call(
                    "get_weather",
                    r#"{"location":"Dublin"}"#,
                    Some("call_1"),
                )],
            ),
            Message::tool_result("sunny, 21C", "call_1", "get_weather"),
        ]);

        assert_eq!(turns.len(), 3, "{turns:?}");
        assert_eq!(turns[1].role, "assistant");
        assert_eq!(
            blocks(&turns[1]),
            json!([{
                "type":"tool_use", "id":"call_1", "name":"get_weather",
                "input":{"location":"Dublin"}
            }])
        );
        assert_eq!(turns[2].role, "user");
        assert_eq!(
            blocks(&turns[2]),
            json!([{"type":"tool_result", "tool_use_id":"call_1", "content":"sunny, 21C"}])
        );
    }

    #[spec("ASTRA-002")]
    #[test]
    fn a_calling_turn_keeps_its_text_beside_the_call() {
        let (_, turns) = convert_messages(vec![
            Message::user("weather?"),
            Message::assistant_with_tool_calls(
                "Checking the forecast.",
                vec![call("get_weather", "{}", Some("call_1"))],
            ),
            Message::tool_result("sunny", "call_1", "get_weather"),
        ]);
        assert_eq!(
            blocks(&turns[1]),
            json!([
                {"type":"text", "text":"Checking the forecast."},
                {"type":"tool_use", "id":"call_1", "name":"get_weather", "input":{}}
            ])
        );
    }

    /// Anthropic reads a trailing assistant message as prefill and continues
    /// it, so a tool result must never travel under that role.
    #[spec("ASTRA-002")]
    #[test]
    fn a_tool_result_is_never_an_assistant_turn() {
        let (_, turns) = convert_messages(vec![
            Message::user("weather?"),
            Message::assistant_with_tool_calls("", vec![call("get_weather", "{}", Some("c1"))]),
            Message::tool_result("sunny, 21C", "c1", "get_weather"),
        ]);
        let last = turns.last().expect("conversation is not empty");
        assert_eq!(last.role, "user");
        assert!(
            serde_json::to_string(&last.content)
                .unwrap()
                .contains("sunny, 21C")
        );
    }

    /// An unanswered `tool_use` block is a 400, and a trailing calling turn is
    /// exactly that shape — the call still has to reach the model as text.
    #[spec("ASTRA-002")]
    #[test]
    fn an_unanswered_call_is_rendered_as_text_with_its_arguments() {
        let (_, turns) = convert_messages(vec![
            Message::user("weather?"),
            Message::assistant_with_tool_calls(
                "",
                vec![call(
                    "get_weather",
                    r#"{"location":"Dublin"}"#,
                    Some("call_1"),
                )],
            ),
        ]);
        assert_eq!(turns.len(), 2, "{turns:?}");
        assert_eq!(turns[1].role, "assistant");
        let text = turns[1].text();
        assert!(text.contains("get_weather"), "{text}");
        assert!(text.contains("Dublin"), "{text}");
        assert!(
            !serde_json::to_string(&turns[1].content)
                .unwrap()
                .contains("tool_use"),
            "an unpaired call must not be sent as a tool_use block"
        );
    }

    /// A partially answered round would leave one call unpaired, so the whole
    /// turn falls back to text rather than sending half a pair.
    #[spec("ASTRA-002")]
    #[test]
    fn a_partly_answered_round_falls_back_for_the_whole_turn() {
        let (_, turns) = convert_messages(vec![
            Message::user("weather and time?"),
            Message::assistant_with_tool_calls(
                "",
                vec![
                    call("get_weather", "{}", Some("c1")),
                    call("get_time", "{}", Some("c2")),
                ],
            ),
            Message::tool_result("sunny", "c1", "get_weather"),
        ]);
        let wire = serde_json::to_string(&turns).unwrap();
        assert!(!wire.contains("tool_use"), "{wire}");
        assert!(!wire.contains("tool_result"), "{wire}");
        assert!(wire.contains("get_time"), "{wire}");
        assert!(wire.contains("sunny"), "{wire}");
    }

    /// A call whose id the provider never assigned cannot be paired, and a
    /// result for a call that was never sent as `tool_use` is a 400.
    #[spec("ASTRA-002")]
    #[test]
    fn calls_and_results_without_ids_stay_text() {
        let (_, turns) = convert_messages(vec![
            Message::user("weather?"),
            Message::assistant_with_tool_calls(
                "",
                vec![call("get_weather", r#"{"city":"Cork"}"#, None)],
            ),
            Message::tool_result("sunny", "", "get_weather"),
        ]);
        let wire = serde_json::to_string(&turns).unwrap();
        assert!(!wire.contains("tool_use"), "{wire}");
        assert!(!wire.contains("tool_result"), "{wire}");
        assert!(wire.contains("Cork"), "{wire}");
        assert!(wire.contains("get_weather"), "{wire}");
    }

    /// Arguments that are not a JSON object cannot be a `tool_use` input, and
    /// substituting an empty object would tell the model it called with none.
    #[spec("ASTRA-002")]
    #[test]
    fn unparseable_arguments_keep_their_text_instead_of_becoming_empty() {
        let (_, turns) = convert_messages(vec![
            Message::user("weather?"),
            Message::assistant_with_tool_calls(
                "",
                vec![call("get_weather", "location=Dublin", Some("c1"))],
            ),
            Message::tool_result("sunny", "c1", "get_weather"),
        ]);
        let wire = serde_json::to_string(&turns).unwrap();
        assert!(!wire.contains("tool_use"), "{wire}");
        assert!(wire.contains("location=Dublin"), "{wire}");
    }

    #[test]
    fn a_call_without_arguments_sends_an_empty_input_object() {
        let (_, turns) = convert_messages(vec![
            Message::user("time?"),
            Message::assistant_with_tool_calls("", vec![call("get_time", "", Some("c1"))]),
            Message::tool_result("noon", "c1", "get_time"),
        ]);
        assert_eq!(
            blocks(&turns[1]),
            json!([{"type":"tool_use", "id":"c1", "name":"get_time", "input":{}}])
        );
    }

    /// Results lead the user turn they share with later prose, which is the
    /// order the API reads them in.
    #[spec("ASTRA-002")]
    #[test]
    fn a_merged_user_turn_leads_with_its_results_and_keeps_its_prose_in_order() {
        let (_, turns) = convert_messages(vec![
            Message::user("weather?"),
            Message::assistant_with_tool_calls("", vec![call("get_weather", "{}", Some("c1"))]),
            Message::tool_result("sunny", "c1", "get_weather"),
            Message::user("and tomorrow?"),
            Message::user("in Cork"),
        ]);
        assert_eq!(turns.len(), 3, "{turns:?}");
        assert_eq!(
            blocks(&turns[2]),
            json!([
                {"type":"tool_result", "tool_use_id":"c1", "content":"sunny"},
                {"type":"text", "text":"and tomorrow?"},
                {"type":"text", "text":"in Cork"}
            ])
        );
    }

    /// A tool that returned nothing must not send an empty content field.
    #[spec("ASTRA-002")]
    #[test]
    fn an_empty_tool_result_sends_no_content_field() {
        let (_, turns) = convert_messages(vec![
            Message::user("run it"),
            Message::assistant_with_tool_calls("", vec![call("run", "{}", Some("c1"))]),
            Message::tool_result("", "c1", "run"),
        ]);
        assert_eq!(
            blocks(&turns[2]),
            json!([{"type":"tool_result", "tool_use_id":"c1"}])
        );
    }

    #[test]
    fn a_conversation_that_does_not_open_on_a_user_turn_gets_one() {
        let (_, turns) = convert_messages(vec![Message::assistant("thinking out loud")]);
        assert_eq!(turns[0].role, "user");
        assert_eq!(turns[0].text(), "Hello");
    }

    #[test]
    fn empty_turns_carry_no_blocks() {
        let (_, turns) = convert_messages(vec![
            Message::user("   "),
            Message::user("real question"),
            Message::assistant(""),
        ]);
        assert_eq!(turns.len(), 1, "{turns:?}");
        assert_eq!(turns[0].text(), "real question");
    }
}
