//! Translate an agent's `MessageDelta` stream into a canonical AG-UI `BaseEvent` stream.

use std::sync::Arc;

use arkavo_agui_protocol::{
    BaseEvent, EventPayload,
    event::{
        RunErrorFields, RunFinishedFields, RunStartedFields, TextMessageContentFields,
        TextMessageEndFields, TextMessageStartFields, ToolCallArgsFields, ToolCallEndFields,
        ToolCallResultFields, ToolCallStartFields,
    },
    input::{RunAgentInput, RunOutcome},
    message::{Message, UserMessageContent},
};
use arkavo_protocol::types::{MessageDelta, MessageDeltaContent, StreamEndReason};
use futures::{Stream, StreamExt};

use crate::agent_connection::AgentConnection;

/// Produce a spec-compliant AG-UI event stream for a single run.
pub fn run_event_stream(
    connection: Arc<AgentConnection>,
    input: RunAgentInput,
) -> impl Stream<Item = BaseEvent> {
    async_stream::stream! {
        let thread_id = input.thread_id.clone();
        let run_id = input.run_id.clone();

        if let Err(e) = connection.wait_until_connected(std::time::Duration::from_secs(10)).await {
            yield run_error(&thread_id, &run_id, "CONNECTION_TIMEOUT", &e.to_string());
            return;
        }

        let session = match connection.open_run_session(None).await {
            Ok(s) => s,
            Err(e) => {
                yield run_error(&thread_id, &run_id, "SESSION_OPEN_FAILED", &e.to_string());
                return;
            }
        };

        if let Some(text) = last_user_text(&input.messages)
            && let Err(e) = connection.send_session_message(&session.session_id, text).await
        {
            yield run_error(&thread_id, &run_id, "MESSAGE_SEND_FAILED", &e.to_string());
            return;
        }

        let mut deltas = match connection.subscribe_message_deltas(&session.session_id).await {
            Ok(s) => s,
            Err(e) => {
                yield run_error(&thread_id, &run_id, "SUBSCRIBE_FAILED", &e.to_string());
                return;
            }
        };

        yield BaseEvent::now(EventPayload::RunStarted(RunStartedFields {
            thread_id: thread_id.clone(),
            run_id: run_id.clone(),
            parent_run_id: input.parent_run_id.clone(),
            input: None,
        }));

        let mut translator = Translator::new(thread_id.clone(), run_id.clone());
        while let Some(delta_result) = deltas.next().await {
            match delta_result {
                Ok(delta) => {
                    for payload in translator.apply(delta) {
                        yield BaseEvent::now(payload);
                    }
                    if translator.finished {
                        return;
                    }
                }
                Err(e) => {
                    yield run_error(&thread_id, &run_id, "STREAM_ERROR", &format!("stream error: {e}"));
                    return;
                }
            }
        }

        for payload in translator.flush() {
            yield BaseEvent::now(payload);
        }

        yield BaseEvent::now(EventPayload::RunFinished(RunFinishedFields {
            thread_id,
            run_id,
            outcome: Some(RunOutcome::Success),
            result: None,
        }));
    }
}

fn run_error(thread_id: &str, run_id: &str, code: &str, message: &str) -> BaseEvent {
    BaseEvent::new(EventPayload::RunError(RunErrorFields {
        thread_id: thread_id.into(),
        run_id: run_id.into(),
        message: message.into(),
        code: Some(code.into()),
    }))
}

fn last_user_text(messages: &[Message]) -> Option<String> {
    messages.iter().rev().find_map(|m| match m {
        Message::User(u) => match &u.content {
            UserMessageContent::Text(t) => Some(t.clone()),
            UserMessageContent::Parts(parts) => parts.iter().find_map(|p| match p {
                arkavo_agui_protocol::message::InputContent::Text(t) => Some(t.text.clone()),
                _ => None,
            }),
        },
        _ => None,
    })
}

struct Translator {
    thread_id: String,
    run_id: String,
    current_message_id: Option<String>,
    current_tool_call_id: Option<String>,
    finished: bool,
}

impl Translator {
    fn new(thread_id: String, run_id: String) -> Self {
        Self {
            thread_id,
            run_id,
            current_message_id: None,
            current_tool_call_id: None,
            finished: false,
        }
    }

    fn apply(&mut self, delta: MessageDelta) -> Vec<EventPayload> {
        let mut out = Vec::new();
        match delta.delta {
            MessageDeltaContent::Text { text } => {
                self.close_tool(&mut out);
                if self.current_message_id.as_ref() != Some(&delta.message_id) {
                    self.close_message(&mut out);
                    self.current_message_id = Some(delta.message_id.clone());
                    out.push(EventPayload::TextMessageStart(TextMessageStartFields {
                        message_id: delta.message_id.clone(),
                        role: "assistant".into(),
                    }));
                }
                out.push(EventPayload::TextMessageContent(TextMessageContentFields {
                    message_id: delta.message_id,
                    delta: text,
                }));
            }
            MessageDeltaContent::ToolCall {
                tool_call_id,
                name,
                args_json_fragment,
                done,
            } => {
                self.close_message(&mut out);
                if self.current_tool_call_id.as_ref() != Some(&tool_call_id) {
                    self.close_tool(&mut out);
                    self.current_tool_call_id = Some(tool_call_id.clone());
                    out.push(EventPayload::ToolCallStart(ToolCallStartFields {
                        tool_call_id: tool_call_id.clone(),
                        tool_call_name: name.unwrap_or_default(),
                        parent_message_id: None,
                    }));
                }
                out.push(EventPayload::ToolCallArgs(ToolCallArgsFields {
                    tool_call_id: tool_call_id.clone(),
                    delta: args_json_fragment,
                }));
                if done {
                    out.push(EventPayload::ToolCallEnd(ToolCallEndFields {
                        tool_call_id,
                    }));
                    self.current_tool_call_id = None;
                }
            }
            MessageDeltaContent::ToolResult {
                tool_call_id,
                content,
                is_error,
            } => {
                self.close_message(&mut out);
                self.close_tool(&mut out);
                out.push(EventPayload::ToolCallResult(ToolCallResultFields {
                    message_id: delta.message_id,
                    tool_call_id,
                    content,
                    role: if is_error { Some("tool".into()) } else { None },
                }));
            }
            MessageDeltaContent::StreamEnd { reason } => {
                self.finished = true;
                out.extend(self.flush());
                match reason {
                    StreamEndReason::Complete => {
                        out.push(EventPayload::RunFinished(RunFinishedFields {
                            thread_id: self.thread_id.clone(),
                            run_id: self.run_id.clone(),
                            outcome: Some(RunOutcome::Success),
                            result: None,
                        }))
                    }
                    other => out.push(EventPayload::RunError(RunErrorFields {
                        thread_id: self.thread_id.clone(),
                        run_id: self.run_id.clone(),
                        message: format!("stream ended: {other:?}"),
                        code: Some("STREAM_END".into()),
                    })),
                }
            }
            MessageDeltaContent::Error { code, message } => {
                self.finished = true;
                out.extend(self.flush());
                out.push(EventPayload::RunError(RunErrorFields {
                    thread_id: self.thread_id.clone(),
                    run_id: self.run_id.clone(),
                    message,
                    code: Some(code),
                }));
            }
            MessageDeltaContent::Metadata { .. } => {}
        }
        out
    }

    fn close_message(&mut self, out: &mut Vec<EventPayload>) {
        if let Some(id) = self.current_message_id.take() {
            out.push(EventPayload::TextMessageEnd(TextMessageEndFields {
                message_id: id,
            }));
        }
    }

    fn close_tool(&mut self, out: &mut Vec<EventPayload>) {
        if let Some(id) = self.current_tool_call_id.take() {
            out.push(EventPayload::ToolCallEnd(ToolCallEndFields {
                tool_call_id: id,
            }));
        }
    }

    fn flush(&mut self) -> Vec<EventPayload> {
        let mut out = Vec::new();
        self.close_message(&mut out);
        self.close_tool(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use arkavo_agui_protocol::message::{AssistantMessage, InputContent, TextContent, UserMessage};

    use super::*;

    #[test]
    fn last_user_text_prefers_plain_text() {
        let messages = vec![Message::User(UserMessage {
            id: "u1".into(),
            content: UserMessageContent::Text("hello".into()),
            name: None,
            encrypted_value: None,
        })];
        assert_eq!(last_user_text(&messages), Some("hello".into()));
    }

    #[test]
    fn last_user_text_extracts_text_from_parts() {
        let messages = vec![Message::User(UserMessage {
            id: "u1".into(),
            content: UserMessageContent::Parts(vec![
                InputContent::Text(TextContent {
                    text: "from part".into(),
                }),
                InputContent::Image(arkavo_agui_protocol::message::ImageContent {
                    source: arkavo_agui_protocol::message::InputContentSource::Url {
                        value: "x".into(),
                        mime_type: None,
                    },
                    metadata: None,
                }),
            ]),
            name: None,
            encrypted_value: None,
        })];
        assert_eq!(last_user_text(&messages), Some("from part".into()));
    }

    #[test]
    fn last_user_text_ignores_non_user_messages() {
        let messages = vec![Message::Assistant(AssistantMessage {
            id: "a1".into(),
            content: Some("hi".into()),
            tool_calls: None,
            name: None,
            encrypted_value: None,
        })];
        assert_eq!(last_user_text(&messages), None);
    }

    #[test]
    fn translator_emits_text_lifecycle() {
        let mut t = Translator::new("t".into(), "r".into());
        let events = t.apply(MessageDelta {
            session_id: "s".into(),
            message_id: "m1".into(),
            sequence: 1,
            delta: MessageDeltaContent::Text {
                text: "hello".into(),
            },
            timestamp: chrono::Utc::now(),
        });
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], EventPayload::TextMessageStart(_)));
        assert!(matches!(events[1], EventPayload::TextMessageContent(_)));

        let events = t.apply(MessageDelta {
            session_id: "s".into(),
            message_id: "m1".into(),
            sequence: 2,
            delta: MessageDeltaContent::Text {
                text: " world".into(),
            },
            timestamp: chrono::Utc::now(),
        });
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], EventPayload::TextMessageContent(_)));

        let events = t.flush();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], EventPayload::TextMessageEnd(_)));
    }

    #[test]
    fn translator_emits_tool_lifecycle() {
        let mut t = Translator::new("t".into(), "r".into());
        let events = t.apply(MessageDelta {
            session_id: "s".into(),
            message_id: "m1".into(),
            sequence: 1,
            delta: MessageDeltaContent::ToolCall {
                tool_call_id: "tc1".into(),
                name: Some("read".into()),
                args_json_fragment: "{\"p".into(),
                done: false,
            },
            timestamp: chrono::Utc::now(),
        });
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], EventPayload::ToolCallStart(_)));
        assert!(matches!(events[1], EventPayload::ToolCallArgs(_)));

        let events = t.apply(MessageDelta {
            session_id: "s".into(),
            message_id: "m1".into(),
            sequence: 2,
            delta: MessageDeltaContent::ToolCall {
                tool_call_id: "tc1".into(),
                name: None,
                args_json_fragment: "\"x\"}".into(),
                done: true,
            },
            timestamp: chrono::Utc::now(),
        });
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], EventPayload::ToolCallArgs(_)));
        assert!(matches!(events[1], EventPayload::ToolCallEnd(_)));
    }
}
