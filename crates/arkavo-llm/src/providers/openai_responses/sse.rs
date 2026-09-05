use super::convert;
use crate::{Error, Result, StreamResponse};
use futures::{Stream, StreamExt};
use serde_json::Value;
use std::collections::VecDeque;
use std::pin::Pin;

const MAX_EVENT_BYTES: usize = 8 * 1024 * 1024;

/// Decode complete events rather than individual network chunks, so fragmented
/// UTF-8, CRLF and multi-line data fields retain their exact meaning.
#[derive(Default)]
struct Decoder {
    pending: Vec<u8>,
    data: String,
    events: VecDeque<String>,
}

impl Decoder {
    fn push(&mut self, bytes: &[u8]) -> Result<()> {
        self.pending.extend_from_slice(bytes);
        let mut consumed = 0;
        while let Some(offset) = self.pending[consumed..].iter().position(|b| *b == b'\n') {
            let end = consumed + offset;
            let line = std::str::from_utf8(&self.pending[consumed..end])
                .map_err(|_| Error::Stream("Invalid UTF-8 in Responses event".into()))?
                .trim_end_matches('\r');
            if line.is_empty() {
                if !self.data.is_empty() {
                    self.events.push_back(std::mem::take(&mut self.data));
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                let data = data.strip_prefix(' ').unwrap_or(data);
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(data);
                if self.data.len() > MAX_EVENT_BYTES {
                    return Err(Error::Stream("Responses event exceeds size limit".into()));
                }
            }
            consumed = end + 1;
        }
        self.pending.drain(..consumed);
        if self.pending.len() > MAX_EVENT_BYTES {
            return Err(Error::Stream("Responses event exceeds size limit".into()));
        }
        Ok(())
    }
}

struct State {
    source: Pin<Box<dyn Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>> + Send>>,
    decoder: Decoder,
    emitted: String,
    terminal: bool,
}

pub(super) fn stream(
    response: reqwest::Response,
) -> Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin> {
    let state = State {
        source: Box::pin(response.bytes_stream()),
        decoder: Decoder::default(),
        emitted: String::new(),
        terminal: false,
    };
    // Pull-based ownership means dropping the consumer immediately drops the HTTP
    // body; there is no detached task continuing to generate billable output.
    Box::new(Box::pin(futures::stream::try_unfold(
        state,
        |mut state| async move {
            if state.terminal {
                return Ok(None);
            }
            loop {
                if let Some(data) = state.decoder.events.pop_front() {
                    if let Some(chunk) = event(&data, &mut state.emitted)? {
                        state.terminal = chunk.done;
                        return Ok(Some((chunk, state)));
                    }
                    continue;
                }
                match state.source.next().await {
                    Some(Ok(bytes)) => state.decoder.push(&bytes)?,
                    Some(Err(error)) => return Err(Error::Request(error.without_url())),
                    None => {
                        return Err(Error::Stream(
                            "Responses stream ended before completion".into(),
                        ));
                    }
                }
            }
        },
    )))
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
                reasoning_content: None,
                done: false,
                inference_timing: None,
                response_items: Vec::new(),
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
                response_items: response.response_items,
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
        Some("error") => Err(Error::Stream("OpenAI Responses stream failed".into())),
        // Wait for the terminal response so refusals retain their billed usage.
        Some("response.refusal.delta" | "response.refusal.done") => Ok(None),
        Some(_) => Ok(None),
        None => Err(Error::Stream("Responses event has no type".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn fragmented_utf8_crlf_and_multiline_data_are_lossless() {
        let input = "data: {\r\ndata: \"type\":\"response.output_text.delta\",\r\ndata: \"delta\":\"🌍\"}\r\n\r\n";
        let mut decoder = Decoder::default();
        for byte in input.as_bytes() {
            decoder.push(&[*byte]).unwrap();
        }
        let mut text = String::new();
        let chunk = event(&decoder.events.pop_front().unwrap(), &mut text)
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
        assert_eq!(chunk.response_items.len(), 1);
    }

    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn missing_completion_and_malformed_events_fail() {
        for data in ["[DONE]", "bad json", r#"{"type":"response.incomplete"}"#] {
            assert!(event(data, &mut String::new()).is_err());
        }
    }

    #[arkavo_test_macros::spec("ASTRA-003")]
    #[test]
    fn invalid_utf8_is_not_replaced() {
        assert!(Decoder::default().push(b"data: \xff\n\n").is_err());
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
        let data = json!({"type":"response.incomplete","response":{"status":"incomplete","output":[],"usage":{"input_tokens":4,"output_tokens":10,"output_tokens_details":{"reasoning_tokens":8}}}}).to_string();
        let error = event(&data, &mut String::new()).unwrap_err();
        let timing = error.inference_timing().unwrap();
        assert_eq!(timing.n_prompt_eval, 4);
        assert_eq!(timing.n_eval, 2);
        assert_eq!(timing.n_thinking_eval, Some(8));
    }
}
