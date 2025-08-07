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
        buffer_size: usize,
    ) -> Self {
        let (tx, rx) = mpsc::channel(buffer_size);

        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut stream = Box::pin(body_stream);
            let mut tool_calls_buffer: std::collections::HashMap<u32, (String, String, String)> =
                std::collections::HashMap::new();

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
                                            if let Some(tool_calls) = &delta.tool_calls {
                                                for tool_call in tool_calls {
                                                    let entry = tool_calls_buffer
                                                        .entry(tool_call.index)
                                                        .or_insert_with(|| {
                                                            (
                                                                String::new(),
                                                                String::new(),
                                                                String::new(),
                                                            )
                                                        });

                                                    if let Some(id) = &tool_call.id {
                                                        entry.0.clone_from(id);
                                                    }
                                                    if let Some(tool_type) = &tool_call.tool_type {
                                                        entry.1.clone_from(tool_type);
                                                    }
                                                    if let Some(function) = &tool_call.function {
                                                        if let Some(name) = &function.name {
                                                            entry.2.clone_from(name);
                                                        }
                                                        if let Some(args) = &function.arguments {
                                                            entry.2.push_str(args);
                                                        }
                                                    }
                                                }
                                            }

                                            // Check if stream is done
                                            if choice.finish_reason.is_some() {
                                                // Tool calls have been aggregated in tool_calls_buffer
                                                // but StreamResponse doesn't support them yet
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
                            .send(Err(KimiError::Stream(format!(
                                "Failed to read SSE stream chunk: {e}"
                            ))))
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
        let trimmed = data.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed == "[DONE]" {
            return None;
        }
        match serde_json::from_str::<StreamChunk>(trimmed) {
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

    #[test]
    fn test_parse_empty_data_line() {
        let line = "data: ";
        assert!(parse_sse_line(line).is_none());
    }

    #[test]
    fn test_parse_whitespace_only_data() {
        let line = "data:    ";
        assert!(parse_sse_line(line).is_none());
    }

    #[test]
    fn test_parse_interleaved_done() {
        // Test parsing when [DONE] appears in the middle of other data
        let lines = [r#"data: {"id":"1","object":"chat.completion.chunk","created":1234567890,"model":"moonshot-v1-8k","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#,
            "data: [DONE]",
            r#"data: {"id":"2","object":"chat.completion.chunk","created":1234567890,"model":"moonshot-v1-8k","choices":[{"index":0,"delta":{"content":"World"},"finish_reason":null}]}"#];

        let results: Vec<_> = lines.iter().map(|line| parse_sse_line(line)).collect();

        assert!(results[0].is_some());
        assert!(results[1].is_none()); // [DONE] returns None
        assert!(results[2].is_some());
    }

    #[test]
    fn test_parse_malformed_chunks() {
        let test_cases = vec![
            ("data: {", "Incomplete JSON"),
            ("data: {\"id\":", "Truncated JSON"),
            ("data: null", "Null data"),
            ("data: []", "Array instead of object"),
            ("data: \"string\"", "String instead of object"),
        ];

        for (line, desc) in test_cases {
            let result = parse_sse_line(line);
            assert!(
                matches!(result, Some(Err(KimiError::Json(_)))),
                "Failed for case: {desc}"
            );
        }
    }

    #[tokio::test]
    async fn test_sse_parser_with_mixed_content() {
        use futures::stream;

        // Simulate a stream with various edge cases
        let chunks = vec![
            Ok(bytes::Bytes::from(
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"moonshot-v1-8k\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Start\"},\"finish_reason\":null}]}\n",
            )),
            Ok(bytes::Bytes::from("\n")),       // Empty line
            Ok(bytes::Bytes::from("data: \n")), // Empty data line
            Ok(bytes::Bytes::from(
                "data: {\"id\":\"2\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"moonshot-v1-8k\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" middle\"},\"finish_reason\":null}]}\n",
            )),
            Ok(bytes::Bytes::from("data: [DONE]\n")),
        ];

        let body_stream = stream::iter(chunks);
        let mut parser = SseParser::new(body_stream, 1024);

        let mut responses = Vec::new();
        while let Some(result) = parser.next().await {
            responses.push(result);
        }

        // Should have received 3 responses: "Start", " middle", and done signal
        assert_eq!(responses.len(), 3);

        // Check first response
        assert!(matches!(&responses[0], Ok(resp) if resp.content == "Start" && !resp.done));

        // Check second response
        assert!(matches!(&responses[1], Ok(resp) if resp.content == " middle" && !resp.done));

        // Check done signal
        assert!(matches!(&responses[2], Ok(resp) if resp.content.is_empty() && resp.done));
    }

    #[tokio::test]
    async fn test_sse_parser_partial_lines() {
        use futures::stream;

        // Simulate chunks that split SSE lines
        let chunks = vec![
            Ok(bytes::Bytes::from(
                "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"moonshot-v1-8k\",\"cho",
            )),
            Ok(bytes::Bytes::from(
                "ices\":[{\"index\":0,\"delta\":{\"content\":\"Split\"},\"finish_reason\":null}]}\n",
            )),
            Ok(bytes::Bytes::from("da")),
            Ok(bytes::Bytes::from("ta: [DO")),
            Ok(bytes::Bytes::from("NE]\n")),
        ];

        let body_stream = stream::iter(chunks);
        let mut parser = SseParser::new(body_stream, 1024);

        let mut responses = Vec::new();
        while let Some(result) = parser.next().await {
            responses.push(result);
        }

        // Should handle split lines correctly
        assert_eq!(responses.len(), 2);
        assert!(matches!(&responses[0], Ok(resp) if resp.content == "Split" && !resp.done));
        assert!(matches!(&responses[1], Ok(resp) if resp.done));
    }
}
