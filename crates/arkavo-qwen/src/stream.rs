use crate::error::{QwenError, Result};
use crate::types::{StreamChunk, ToolCall};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamResponse {
    pub content: String,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

pub struct SseStream {
    rx: mpsc::Receiver<Result<StreamResponse>>,
}

impl SseStream {
    pub fn new(body: impl Stream<Item = reqwest::Result<Bytes>> + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut stream = Box::pin(body);

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        while let Some(newline_pos) = buffer.find('\n') {
                            let line = buffer[..newline_pos].trim().to_string();
                            buffer.drain(..=newline_pos);

                            if line.is_empty() {
                                continue;
                            }

                            if let Some(data) = line.strip_prefix("data: ") {
                                if data.trim() == "[DONE]" {
                                    let _ = tx
                                        .send(Ok(StreamResponse {
                                            content: String::new(),
                                            done: true,
                                            tool_calls: None,
                                        }))
                                        .await;
                                    return;
                                }

                                match serde_json::from_str::<StreamChunk>(data) {
                                    Ok(chunk) => {
                                        for choice in chunk.choices {
                                            if let Some(content) = choice.delta.content {
                                                let response = StreamResponse {
                                                    content,
                                                    done: choice.finish_reason.is_some(),
                                                    tool_calls: choice.delta.tool_calls,
                                                };

                                                if tx.send(Ok(response)).await.is_err() {
                                                    return;
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        let _ = tx
                                            .send(Err(QwenError::StreamError {
                                                message: format!("Failed to parse chunk: {e}"),
                                            }))
                                            .await;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(QwenError::NetworkError(e))).await;
                        return;
                    }
                }
            }
        });

        Self { rx }
    }
}

impl Stream for SseStream {
    type Item = Result<StreamResponse>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;
    use futures::stream;

    #[tokio::test]
    async fn test_sse_stream_simple_content() {
        let data = r#"data: {"id":"1","object":"chat.completion.chunk","created":1234567890,"model":"qwen3-32b","choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"id":"1","object":"chat.completion.chunk","created":1234567890,"model":"qwen3-32b","choices":[{"index":0,"delta":{"content":" world"},"finish_reason":null}]}

data: [DONE]
"#;

        let body_stream = stream::once(async move { Ok(Bytes::from(data)) });
        let mut sse_stream = SseStream::new(body_stream);

        let mut responses = Vec::new();
        while let Some(result) = sse_stream.next().await {
            responses.push(result.unwrap());
        }

        assert_eq!(responses.len(), 3);
        assert_eq!(responses[0].content, "Hello");
        assert!(!responses[0].done);
        assert_eq!(responses[1].content, " world");
        assert!(!responses[1].done);
        assert!(responses[2].done);
    }

    #[tokio::test]
    async fn test_sse_stream_empty_lines() {
        let data = "data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1234567890,\"model\":\"qwen3-32b\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Test\"},\"finish_reason\":null}]}\n\n\ndata: [DONE]\n";

        let body_stream = stream::once(async move { Ok(Bytes::from(data)) });
        let mut sse_stream = SseStream::new(body_stream);

        let mut responses = Vec::new();
        while let Some(result) = sse_stream.next().await {
            responses.push(result.unwrap());
        }

        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0].content, "Test");
        assert!(responses[1].done);
    }

    #[tokio::test]
    async fn test_sse_stream_invalid_json() {
        let data = "data: {invalid json}\ndata: [DONE]\n";

        let body_stream = stream::once(async move { Ok(Bytes::from(data)) });
        let mut sse_stream = SseStream::new(body_stream);

        let mut got_error = false;
        while let Some(result) = sse_stream.next().await {
            if result.is_err() {
                got_error = true;
            }
        }

        assert!(got_error);
    }

    #[tokio::test]
    async fn test_sse_stream_immediate_done() {
        let data = "data: [DONE]\n";

        let body_stream = stream::once(async move { Ok(Bytes::from(data)) });
        let mut sse_stream = SseStream::new(body_stream);

        let mut responses = Vec::new();
        while let Some(result) = sse_stream.next().await {
            responses.push(result.unwrap());
        }

        assert_eq!(responses.len(), 1);
        assert!(responses[0].done);
        assert!(responses[0].content.is_empty());
    }
}
