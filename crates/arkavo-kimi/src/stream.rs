use crate::error::{KimiError, Result};
use crate::provider::StreamResponse;
use crate::types::StreamChunk;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;
use tracing::warn;

/// SSE (Server-Sent Events) parser for Kimi streaming responses
pub struct SseParser {
    receiver: mpsc::Receiver<Result<StreamResponse>>,
}

impl SseParser {
    /// Create a new SSE parser from a response body stream
    pub fn new(
        body_stream: impl Stream<Item = reqwest::Result<bytes::Bytes>> + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel(1024);

        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut stream = Box::pin(body_stream);
            // Future use: tool calls could be buffered here for aggregation
            // let mut tool_calls_buffer: Vec<DeltaToolCall> = Vec::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        // Process complete lines
                        while let Some(newline_pos) = buffer.find('\n') {
                            let line = buffer[..newline_pos].trim().to_string();
                            buffer.drain(..=newline_pos);

                            // Skip empty lines
                            if line.is_empty() {
                                continue;
                            }

                            // Process SSE data lines
                            if let Some(data) = line.strip_prefix("data: ") {
                                if data == "[DONE]" {
                                    // Send final done message
                                    let _ = tx
                                        .send(Ok(StreamResponse {
                                            content: String::new(),
                                            done: true,
                                        }))
                                        .await;
                                    return;
                                }

                                // Parse the JSON chunk
                                match serde_json::from_str::<StreamChunk>(data) {
                                    Ok(chunk) => {
                                        for choice in chunk.choices {
                                            let delta = &choice.delta;

                                            // Handle content delta
                                            if let Some(content) = &delta.content {
                                                if !content.is_empty() {
                                                    let _ = tx
                                                        .send(Ok(StreamResponse {
                                                            content: content.clone(),
                                                            done: false,
                                                        }))
                                                        .await;
                                                }
                                            }

                                            // Handle tool calls delta
                                            if let Some(_tool_calls) = &delta.tool_calls {
                                                // Tool calls can be processed here
                                                // For now, we focus on text content streaming
                                            }

                                            // Check if stream is done
                                            if choice.finish_reason.is_some() {
                                                let _ = tx
                                                    .send(Ok(StreamResponse {
                                                        content: String::new(),
                                                        done: true,
                                                    }))
                                                    .await;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to parse SSE chunk: {} - Data: {}", e, data);
                                        // Continue processing other chunks
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(KimiError::Stream(format!("Stream read error: {e}"))))
                            .await;
                        return;
                    }
                }
            }

            // Handle any remaining data in buffer
            if !buffer.trim().is_empty() {
                warn!("Incomplete SSE data in buffer: {}", buffer);
            }
        });

        Self { receiver: rx }
    }
}

impl Stream for SseParser {
    type Item = Result<StreamResponse>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}

/// Parse a single SSE line for testing
#[cfg(test)]
pub fn parse_sse_line(line: &str) -> Option<Result<StreamChunk>> {
    if let Some(data) = line.strip_prefix("data: ") {
        if data == "[DONE]" {
            return None;
        }
        match serde_json::from_str::<StreamChunk>(data) {
            Ok(chunk) => Some(Ok(chunk)),
            Err(e) => Some(Err(KimiError::Json(e))),
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sse_done() {
        let line = "data: [DONE]";
        assert!(parse_sse_line(line).is_none());
    }

    #[test]
    fn test_parse_sse_content() {
        let line = r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1234567890,"model":"moonshot-v1-8k","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;

        match parse_sse_line(line) {
            Some(Ok(chunk)) => {
                assert_eq!(chunk.choices.len(), 1);
                assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("Hello"));
                assert!(chunk.choices[0].finish_reason.is_none());
            }
            _ => panic!("Failed to parse valid SSE line"),
        }
    }

    #[test]
    fn test_parse_sse_invalid() {
        let line = "data: {invalid json}";
        assert!(matches!(
            parse_sse_line(line),
            Some(Err(KimiError::Json(_)))
        ));
    }

    #[test]
    fn test_parse_non_sse_line() {
        let line = "not an SSE line";
        assert!(parse_sse_line(line).is_none());
    }
}
