use super::convert;
use crate::common::sse::EventStream;
use crate::{Error, Result, StreamResponse};
use futures::Stream;
use serde_json::Value;
use std::time::{Duration, Instant};

struct State {
    events: EventStream,
    emitted: String,
    terminal: bool,
    started: Instant,
    /// When the first visible token arrived: the boundary between the model's
    /// deliberation and its generation.
    first_output: Option<Instant>,
}

pub(super) fn stream(
    response: reqwest::Response,
    idle: Duration,
) -> Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin> {
    let state = State {
        events: EventStream::new(response.bytes_stream(), idle),
        emitted: String::new(),
        terminal: false,
        started: Instant::now(),
        first_output: None,
    };
    // Pull-based ownership means dropping the consumer immediately drops the HTTP
    // body; there is no detached task continuing to generate billable output.
    Box::new(Box::pin(futures::stream::try_unfold(state, step)))
}

/// Read events until one of them produces a chunk for the caller.
///
/// A body that ends without `response.completed` never delivered the answer the
/// caller was billed for, so it is a failure rather than an end of turn.
async fn step(mut state: State) -> Result<Option<(StreamResponse, State)>> {
    if state.terminal {
        return Ok(None);
    }
    loop {
        let Some(data) = state.events.next_event().await? else {
            return Err(Error::Stream(
                "Responses stream ended before completion".into(),
            ));
        };
        if let Some(mut chunk) = event(&data, &mut state.emitted)? {
            state.terminal = chunk.done;
            if chunk.done {
                attribute(&mut chunk, &state);
            } else if state.first_output.is_none() {
                state.first_output = Some(Instant::now());
            }
            return Ok(Some((chunk, state)));
        }
    }
}

/// Split the measured wall time at the first visible token.
///
/// The provider reports token counts but no latencies, so the only honest
/// prefill figure is the time the caller actually waited for the first token;
/// everything after it is generation. With no visible token at all the whole
/// wait was prefill.
fn attribute(chunk: &mut StreamResponse, state: &State) {
    let Some(timing) = chunk.inference_timing.as_mut() else {
        return;
    };
    let first = state.first_output.unwrap_or_else(Instant::now);
    timing.prompt_eval_ms = first.duration_since(state.started).as_secs_f64() * 1000.0;
    timing.generation_ms = first.elapsed().as_secs_f64() * 1000.0;
}

fn event(data: &str, emitted: &mut String) -> Result<Option<StreamResponse>> {
    if data == "[DONE]" {
        return Err(Error::Stream(
            "Responses stream ended without a completed response".into(),
        ));
    }
    let value: Value = serde_json::from_str(data)
        .map_err(|_| Error::Stream("Malformed Responses event".into()))?;
    match value["type"].as_str() {
        Some("response.output_text.delta") => {
            let delta = value["delta"]
                .as_str()
                .ok_or_else(|| Error::Stream("Responses text delta is missing".into()))?;
            emitted.push_str(delta);
            Ok(Some(StreamResponse {
                content: delta.into(),
                ..Default::default()
            }))
        }
        Some("response.completed") => {
            let response = convert::response(value["response"].clone())?;
            if !response.tool_calls.is_empty() {
                return Err(Error::Stream(
                    "Unexpected function call in text-only Responses stream".into(),
                )
                .with_inference_timing(response.inference_timing));
            }
            let tail = response
                .content
                .strip_prefix(emitted.as_str())
                .ok_or_else(|| {
                    Error::Stream("Responses final text differs from streamed text".into())
                        .with_inference_timing(response.inference_timing.clone())
                })?;
            Ok(Some(StreamResponse {
                content: tail.into(),
                reasoning_content: None,
                done: true,
                inference_timing: response.inference_timing,
                provider_state: response.provider_state,
            }))
        }
        Some("response.failed" | "response.incomplete") => {
            match convert::response(value["response"].clone()) {
                Err(error) => Err(error),
                Ok(_) => Err(Error::Stream(
                    "Unexpected completed response in failure event".into(),
                )),
            }
        }
        // The event's own `type` is just "error"; its `code` is the reason. The
        // message text beside it is never read.
        Some("error") => Err(Error::provider_refusal(
            value["code"].as_str(),
            None,
            "stream_error",
        )),
        // Wait for the terminal response so refusals retain their billed usage.
        Some("response.refusal.delta" | "response.refusal.done") => Ok(None),
        Some(_) => Ok(None),
        None => Err(Error::Stream("Responses event has no type".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use serde_json::json;

    /// The shared decoder frames the events; this pins that the provider's
    /// stream still reads them byte-for-byte through it.
    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn fragmented_utf8_crlf_and_multiline_data_reach_the_event_handler() {
        let input = "data: {\r\ndata: \"type\":\"response.output_text.delta\",\r\ndata: \"delta\":\"🌍\"}\r\n\r\n";
        let mut decoder = crate::common::sse::Decoder::default();
        for byte in input.as_bytes() {
            decoder.push(&[*byte]).unwrap();
        }
        let mut text = String::new();
        let chunk = event(&decoder.next_event().unwrap(), &mut text)
            .unwrap()
            .unwrap();
        assert_eq!(chunk.content, "🌍");
        assert_eq!(text, "🌍");
    }

    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn completed_emits_unstreamed_tail_once_and_preserves_state() {
        let data = json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","content":[{"type":"output_text","text":"hello"}]}]}}).to_string();
        let chunk = event(&data, &mut "hel".into()).unwrap().unwrap();
        assert_eq!(chunk.content, "lo");
        assert!(chunk.done);
        assert_eq!(
            chunk
                .provider_state
                .replay_items_for(crate::ProviderStateTag::OpenAiResponses)
                .map(|items| items.len()),
            Some(1)
        );
    }

    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn missing_completion_and_malformed_events_fail() {
        for data in ["[DONE]", "bad json", r#"{"type":"response.incomplete"}"#] {
            assert!(event(data, &mut String::new()).is_err());
        }
    }

    /// A body that stops before `response.completed` never delivered the text
    /// the caller was billed for, and must not read as a clean end of turn.
    #[arkavo_test_macros::spec("ASTRA-003")]
    #[tokio::test]
    async fn a_body_that_ends_before_completion_fails() {
        let state = State {
            events: EventStream::new(
                futures::stream::iter(vec![Ok(bytes::Bytes::from_static(
                    b"data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n",
                ))]),
                Duration::from_secs(1),
            ),
            emitted: String::new(),
            terminal: false,
            started: Instant::now(),
            first_output: None,
        };
        let mut stream = Box::pin(futures::stream::try_unfold(state, step));
        assert_eq!(stream.next().await.unwrap().unwrap().content, "hi");
        let error = stream.next().await.unwrap().unwrap_err();
        assert!(
            error.to_string().contains("ended before completion"),
            "{error}"
        );
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use serde_json::json;

    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn terminal_failure_preserves_usage_and_refusal_delta_stays_private() {
        assert!(
            event(
                r#"{"type":"response.refusal.delta","delta":"refused"}"#,
                &mut String::new()
            )
            .unwrap()
            .is_none()
        );
        let data = json!({"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[],"usage":{"input_tokens":4,"output_tokens":10,"output_tokens_details":{"reasoning_tokens":8}}}}).to_string();
        let error = event(&data, &mut String::new()).unwrap_err();
        let timing = error.inference_timing().unwrap();
        assert_eq!(timing.n_prompt_eval, 4);
        assert_eq!(timing.n_eval, 2);
        assert_eq!(timing.n_thinking_eval, Some(8));
        assert_eq!(
            error.provider_code(),
            Some("max_output_tokens"),
            "a truncation must stay distinguishable from a policy refusal"
        );
    }

    /// The stream's own error event is the only report of a mid-stream failure;
    /// its code has to survive, and the message beside it must not.
    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn stream_error_event_reports_its_code_only() {
        let error = event(
            r#"{"type":"error","code":"rate_limit_exceeded","message":"prompt-canary"}"#,
            &mut String::new(),
        )
        .unwrap_err();
        assert_eq!(error.provider_code(), Some("rate_limit_exceeded"));
        assert!(!error.to_string().contains("prompt-canary"));

        let error = event(
            r#"{"type":"error","message":"prompt-canary"}"#,
            &mut String::new(),
        )
        .unwrap_err();
        assert_eq!(error.provider_code(), Some("stream_error"));
        assert!(!error.to_string().contains("prompt-canary"));
    }

    /// The perf line reports what the caller waited for: everything up to the
    /// first token is prefill, the rest is generation.
    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn terminal_timing_splits_wall_time_at_the_first_token() {
        let mut state = State {
            events: EventStream::new(futures::stream::empty(), Duration::from_secs(1)),
            emitted: String::new(),
            terminal: false,
            started: Instant::now()
                .checked_sub(Duration::from_millis(80))
                .expect("fixture clock"),
            first_output: Some(
                Instant::now()
                    .checked_sub(Duration::from_millis(30))
                    .expect("fixture clock"),
            ),
        };
        let mut chunk = StreamResponse {
            done: true,
            inference_timing: Some(crate::InferenceTiming::default()),
            ..Default::default()
        };
        attribute(&mut chunk, &state);
        let timing = chunk.inference_timing.clone().unwrap();
        assert!(timing.prompt_eval_ms >= 40.0, "{timing:?}");
        assert!(timing.generation_ms >= 25.0, "{timing:?}");

        // With no visible token the whole wait was deliberation.
        state.first_output = None;
        let mut chunk = StreamResponse {
            done: true,
            inference_timing: Some(crate::InferenceTiming::default()),
            ..Default::default()
        };
        attribute(&mut chunk, &state);
        let timing = chunk.inference_timing.unwrap();
        assert!(timing.prompt_eval_ms >= 75.0, "{timing:?}");
        assert!(timing.generation_ms < 5.0, "{timing:?}");
    }
}
