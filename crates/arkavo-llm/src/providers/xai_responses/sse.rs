//! Event handling for the xAI Responses stream.
//!
//! Framing, the stall bound and ownership of the HTTP body come from
//! [`crate::common::sse`]; this module only decides what each event means.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::Stream;
use serde_json::Value;

use crate::common::sse::EventStream;
use crate::{Error, Result, StreamResponse};

use super::types::timing_from_usage;

struct State {
    events: EventStream,
    terminal: bool,
    last_response_id: Arc<Mutex<Option<String>>>,
}

pub(super) fn stream(
    response: reqwest::Response,
    idle: Duration,
    last_response_id: Arc<Mutex<Option<String>>>,
) -> Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin> {
    let state = State {
        events: EventStream::new(response.bytes_stream(), idle),
        terminal: false,
        last_response_id,
    };
    // Pull-based ownership: dropping the consumer drops the body, so a cancelled
    // read stops the generation the caller is billed for instead of detaching it.
    Box::new(Box::pin(futures::stream::try_unfold(state, step)))
}

/// Read events until one of them produces a chunk for the caller.
///
/// The first terminal chunk ends the stream, so a `response.completed` followed
/// by `[DONE]` signals once. Unlike the OpenAI Responses stream, a body that
/// ends without a terminal event is not an error here: xAI closes some streams
/// after the last delta, and the deltas already delivered are the answer.
async fn step(mut state: State) -> Result<Option<(StreamResponse, State)>> {
    if state.terminal {
        return Ok(None);
    }
    loop {
        let Some(data) = state.events.next_event().await? else {
            return Ok(None);
        };
        let last_response_id = Arc::clone(&state.last_response_id);
        let chunk = event(&data, &mut |id| {
            if let Ok(mut slot) = last_response_id.lock() {
                *slot = Some(id);
            }
        })?;
        if let Some(chunk) = chunk {
            state.terminal = chunk.done;
            return Ok(Some((chunk, state)));
        }
    }
}

/// Interpret one complete event payload.
///
/// `Ok(None)` is an event this build does not render — including a payload it
/// cannot parse, which xAI emits for keep-alives and future event types and
/// which must not fail a turn that is otherwise delivering text.
fn event(data: &str, on_response_id: &mut dyn FnMut(String)) -> Result<Option<StreamResponse>> {
    if data == "[DONE]" {
        return Ok(Some(StreamResponse {
            done: true,
            ..Default::default()
        }));
    }
    let Ok(event) = serde_json::from_str::<Value>(data) else {
        return Ok(None);
    };
    let delta = |key: &str| event.get(key).and_then(Value::as_str).map(str::to_string);
    match event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "response.output_text.delta" => Ok(delta("delta").map(|content| StreamResponse {
            content,
            ..Default::default()
        })),
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            Ok(delta("delta").map(|reasoning| StreamResponse {
                reasoning_content: Some(reasoning),
                ..Default::default()
            }))
        }
        "response.completed" => {
            if let Some(id) = event.pointer("/response/id").and_then(Value::as_str) {
                on_response_id(id.to_string());
            }
            Ok(Some(StreamResponse {
                done: true,
                inference_timing: event.pointer("/response/usage").and_then(timing_from_usage),
                ..Default::default()
            }))
        }
        "response.failed" => Err(Error::Provider(
            event
                .pointer("/response/error")
                .map_or_else(|| "response.failed".to_string(), Value::to_string),
        )),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    async fn read(chunks: &[&'static [u8]]) -> Vec<Result<StreamResponse>> {
        let body: Vec<reqwest::Result<bytes::Bytes>> = chunks
            .iter()
            .map(|chunk| Ok(bytes::Bytes::from_static(chunk)))
            .collect();
        let state = State {
            events: EventStream::new(futures::stream::iter(body), Duration::from_secs(1)),
            terminal: false,
            last_response_id: Arc::new(Mutex::new(None)),
        };
        futures::stream::try_unfold(state, step).collect().await
    }

    /// A multi-byte character split across TCP chunks must arrive whole, and a
    /// CRLF frame must not leave a carriage return in the payload.
    #[tokio::test]
    async fn utf8_split_across_chunks_and_crlf_framing_survive_the_decoder() {
        let chunks: &[&[u8]] = &[
            b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"\xe4\xbd",
            b"\xa0\"}\r\n\r\ndata: [DONE]\r\n\r\n",
        ];
        let chunks = read(chunks).await;
        assert_eq!(chunks.len(), 2, "{chunks:?}");
        assert_eq!(chunks[0].as_ref().unwrap().content, "你");
        assert!(chunks[1].as_ref().unwrap().done);
    }

    /// `response.completed` and the `[DONE]` that follows it are one end of turn.
    #[tokio::test]
    async fn the_terminal_signal_is_emitted_once() {
        let chunks = read(&[
            b"data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"usage\":{\"input_tokens\":1,\"output_tokens\":2}}}\n\n",
            b"data: [DONE]\n\ndata: [DONE]\n\n",
        ])
        .await;
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        let chunk = chunks[0].as_ref().unwrap();
        assert!(chunk.done);
        assert_eq!(chunk.inference_timing.as_ref().unwrap().n_eval, 2);
    }

    #[test]
    fn the_completed_event_records_the_response_id() {
        let mut id = None;
        let chunk = event(
            r#"{"type":"response.completed","response":{"id":"resp_7"}}"#,
            &mut |seen| id = Some(seen),
        )
        .unwrap()
        .unwrap();
        assert!(chunk.done);
        assert_eq!(id.as_deref(), Some("resp_7"));
    }

    #[test]
    fn reasoning_deltas_stay_out_of_the_visible_content() {
        let chunk = event(
            r#"{"type":"response.reasoning_text.delta","delta":"weighing"}"#,
            &mut |_| {},
        )
        .unwrap()
        .unwrap();
        assert!(chunk.content.is_empty());
        assert_eq!(chunk.reasoning_content.as_deref(), Some("weighing"));
    }

    /// Keep-alives and event types this build does not render must not end or
    /// fail a turn that is still delivering text.
    #[test]
    fn unparseable_and_unknown_events_are_passed_over() {
        for data in [
            "not json",
            r#"{"type":"response.in_progress"}"#,
            r#"{"type":"response.output_text.delta"}"#,
        ] {
            assert!(event(data, &mut |_| {}).unwrap().is_none(), "{data}");
        }
    }

    #[test]
    fn a_failed_response_reports_the_provider_error() {
        let error = event(
            r#"{"type":"response.failed","response":{"error":{"code":"overloaded"}}}"#,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(error.to_string().contains("overloaded"), "{error}");
    }

    /// xAI closes some streams after the last delta; the text already delivered
    /// is the answer, so the end of the body is the end of the stream.
    #[tokio::test]
    async fn a_body_that_ends_without_a_terminal_event_ends_the_stream() {
        let chunks =
            read(&[b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n"]).await;
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        assert_eq!(chunks[0].as_ref().unwrap().content, "hi");
    }

    /// An undecodable byte must fail rather than reach the model's caller as a
    /// replacement character the endpoint never sent.
    #[tokio::test]
    async fn invalid_utf8_fails_the_stream() {
        let chunks = read(&[b"data: \xff\n\n"]).await;
        assert!(chunks[0].is_err(), "{chunks:?}");
    }
}
