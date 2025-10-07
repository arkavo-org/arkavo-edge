use crate::error::{GeminiError, Result};
use crate::types::{FunctionCall, StreamChunk, StreamResponse};
use bytes::Bytes;
use futures::Stream;
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub struct GeminiSseStream {
    rx: mpsc::Receiver<Result<StreamResponse>>,
}

impl GeminiSseStream {
    pub fn new(body: impl Stream<Item = reqwest::Result<Bytes>> + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            let mut stream = Box::pin(body);
            let mut buffer = String::new();
            let mut accumulated_text = String::new();
            let mut accumulated_calls: Vec<FunctionCall> = Vec::new();

            while let Some(chunk_result) = futures::StreamExt::next(&mut stream).await {
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
                                debug!("SSE data: {}", data);

                                match serde_json::from_str::<StreamChunk>(data) {
                                    Ok(chunk) => {
                                        for candidate in chunk.candidates {
                                            let finish_reason = candidate.finish_reason.clone();

                                            for part in candidate.content.parts {
                                                match part {
                                                    crate::types::StreamPart::Text { text } => {
                                                        accumulated_text.push_str(&text);
                                                    }
                                                    crate::types::StreamPart::FunctionCall {
                                                        function_call,
                                                    } => {
                                                        accumulated_calls.push(FunctionCall {
                                                            name: function_call.name,
                                                            args: function_call.args,
                                                            id: format!(
                                                                "call-{}",
                                                                uuid::Uuid::new_v4()
                                                            ),
                                                        });
                                                    }
                                                }
                                            }

                                            let done = finish_reason.is_some();
                                            let response = StreamResponse {
                                                text: if accumulated_text.is_empty() {
                                                    None
                                                } else {
                                                    Some(accumulated_text.clone())
                                                },
                                                function_calls: accumulated_calls.clone(),
                                                done,
                                            };

                                            if tx.send(Ok(response)).await.is_err() {
                                                debug!("Receiver dropped");
                                                return;
                                            }

                                            if done {
                                                return;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to parse SSE chunk: {} - data: {}", e, data);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(Err(GeminiError::ApiError(format!("Stream error: {e}"))))
                            .await;
                        return;
                    }
                }
            }

            let final_response = StreamResponse {
                text: if accumulated_text.is_empty() {
                    None
                } else {
                    Some(accumulated_text)
                },
                function_calls: accumulated_calls,
                done: true,
            };
            let _ = tx.send(Ok(final_response)).await;
        });

        Self { rx }
    }

    pub async fn next(&mut self) -> Option<Result<StreamResponse>> {
        self.rx.recv().await
    }
}

impl Stream for GeminiSseStream {
    type Item = Result<StreamResponse>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}
