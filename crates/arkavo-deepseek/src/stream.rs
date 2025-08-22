use crate::error::{DeepSeekError, Result};
use crate::types::{Delta, DeltaToolCall, StreamChunk};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;

/// Stream response type
#[derive(Debug, Clone)]
pub struct StreamResponse {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<DeltaToolCall>>,
    pub done: bool,
}

/// SSE event parser for DeepSeek streaming
pub struct SseStream {
    rx: mpsc::Receiver<Result<StreamResponse>>,
}

impl SseStream {
    /// Create a new SSE stream from a response body
    pub fn new(body: impl Stream<Item = reqwest::Result<Bytes>> + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut stream = Box::pin(body);
            let mut accumulated_tool_calls: Vec<DeltaToolCall> = Vec::new();

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        // Process complete lines
                        while let Some(newline_pos) = buffer.find('\n') {
                            let line = buffer[..newline_pos].trim().to_string();
                            buffer.drain(..=newline_pos);

                            if line.is_empty() {
                                continue;
                            }

                            // Handle SSE data lines
                            if let Some(data) = line.strip_prefix("data: ") {
                                if data.trim() == "[DONE]" {
                                    // Send final response with accumulated tool calls
                                    if !accumulated_tool_calls.is_empty() {
                                        let _ = tx
                                            .send(Ok(StreamResponse {
                                                content: None,
                                                reasoning_content: None,
                                                tool_calls: Some(accumulated_tool_calls.clone()),
                                                done: false,
                                            }))
                                            .await;
                                    }
                                    let _ = tx
                                        .send(Ok(StreamResponse {
                                            content: None,
                                            reasoning_content: None,
                                            tool_calls: None,
                                            done: true,
                                        }))
                                        .await;
                                    return;
                                }

                                // Parse JSON chunk
                                match serde_json::from_str::<StreamChunk>(data) {
                                    Ok(chunk) => {
                                        for choice in chunk.choices {
                                            let response = process_delta(
                                                choice.delta,
                                                &mut accumulated_tool_calls,
                                            );
                                            if let Some(resp) = response
                                                && tx.send(Ok(resp)).await.is_err()
                                            {
                                                return; // Receiver dropped
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx
                                            .send(Err(DeepSeekError::StreamError {
                                                message: format!("Failed to parse chunk: {e}"),
                                            }))
                                            .await;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(DeepSeekError::NetworkError {
                                message: e.to_string(),
                            }))
                            .await;
                        return;
                    }
                }
            }
        });

        Self { rx }
    }
}

/// Process a delta and accumulate tool calls
fn process_delta(
    delta: Delta,
    accumulated_tool_calls: &mut Vec<DeltaToolCall>,
) -> Option<StreamResponse> {
    // Handle tool calls
    if let Some(tool_calls) = delta.tool_calls {
        for delta_call in tool_calls {
            // Find or create the tool call at this index
            if let Some(existing) = accumulated_tool_calls
                .iter_mut()
                .find(|tc| tc.index == delta_call.index)
            {
                // Update existing tool call
                if delta_call.id.is_some() {
                    existing.id = delta_call.id;
                }
                if delta_call.tool_type.is_some() {
                    existing.tool_type = delta_call.tool_type;
                }
                if let Some(func) = delta_call.function {
                    if existing.function.is_none() {
                        existing.function = Some(func);
                    } else if let Some(ref mut existing_func) = existing.function {
                        // Merge function data
                        if func.name.is_some() {
                            existing_func.name = func.name;
                        }
                        if let Some(new_args) = func.arguments {
                            if let Some(ref mut args) = existing_func.arguments {
                                args.push_str(&new_args);
                            } else {
                                existing_func.arguments = Some(new_args);
                            }
                        }
                    }
                }
            } else {
                // Add new tool call
                accumulated_tool_calls.push(delta_call);
            }
        }
        // Don't emit tool calls until complete
        return None;
    }

    // Handle content
    if delta.content.is_some() || delta.reasoning_content.is_some() {
        return Some(StreamResponse {
            content: delta.content,
            reasoning_content: delta.reasoning_content,
            tool_calls: None,
            done: false,
        });
    }

    None
}

impl Stream for SseStream {
    type Item = Result<StreamResponse>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    #[tokio::test]
    async fn test_sse_stream_simple_content() {
        let data = r#"data: {"id":"1","object":"chat.completion.chunk","created":1234567890,"model":"deepseek-chat","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"1","object":"chat.completion.chunk","created":1234567890,"model":"deepseek-chat","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}

data: [DONE]
"#;

        let body_stream = stream::once(async move { Ok(Bytes::from(data)) });
        let mut sse_stream = SseStream::new(body_stream);

        let mut responses = Vec::new();
        while let Some(result) = sse_stream.next().await {
            responses.push(result.unwrap());
        }

        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0].content, Some("Hello".to_string()));
        assert_eq!(responses[1].content, Some(" world".to_string()));
        assert!(responses[2].done);
    }

    #[tokio::test]
    async fn test_sse_stream_with_reasoning() {
        let data = r#"data: {"id":"1","object":"chat.completion.chunk","created":1234567890,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{"reasoning_content":"Let me think..."},"finish_reason":null}]}

data: {"id":"1","object":"chat.completion.chunk","created":1234567890,"model":"deepseek-reasoner","choices":[{"index":0,"delta":{"content":"The answer is 4"},"finish_reason":null}]}

data: [DONE]
"#;

        let body_stream = stream::once(async move { Ok(Bytes::from(data)) });
        let mut sse_stream = SseStream::new(body_stream);

        let mut responses = Vec::new();
        while let Some(result) = sse_stream.next().await {
            responses.push(result.unwrap());
        }

        assert_eq!(
            responses[0].reasoning_content,
            Some("Let me think...".to_string())
        );
        assert_eq!(responses[1].content, Some("The answer is 4".to_string()));
        assert!(responses[2].done);
    }
}
