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
            let mut event_data = String::new();
            let mut accumulated_text = String::new();
            let mut last_sent_len = 0;
            let mut accumulated_calls: Vec<FunctionCall> = Vec::new();

            while let Some(chunk_result) = futures::StreamExt::next(&mut stream).await {
                match chunk_result {
                    Ok(bytes) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        while let Some(newline_pos) = buffer.find('\n') {
                            let line = buffer[..newline_pos].trim().to_string();
                            buffer.drain(..=newline_pos);

                            if line.is_empty() {
                                if !event_data.is_empty() {
                                    let data = event_data.trim();
                                    debug!("Complete SSE event ({} bytes): {}", data.len(),
                                           if data.len() > 200 {
                                               format!("{}...{}", &data[..100], &data[data.len()-100..])
                                           } else {
                                               data.to_string()
                                           });

                                    match serde_json::from_str::<StreamChunk>(data) {
                                        Ok(chunk) => {
                                        let mut new_text = String::new();
                                        let mut finish_reason = None;

                                        for candidate in chunk.candidates {
                                            finish_reason.clone_from(&candidate.finish_reason);

                                            for part in candidate.content.parts {
                                                match part {
                                                    crate::types::StreamPart::Text { text } => {
                                                        if !text.is_empty() {
                                                            new_text.push_str(&text);
                                                            accumulated_text.push_str(&text);
                                                        }
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
                                        }

                                        if !new_text.is_empty() || !accumulated_calls.is_empty() || finish_reason.is_some() {
                                            let done = finish_reason.is_some();
                                            let response = StreamResponse {
                                                text: if new_text.is_empty() {
                                                    None
                                                } else {
                                                    Some(new_text)
                                                },
                                                function_calls: accumulated_calls.clone(),
                                                done,
                                            };

                                            if tx.send(Ok(response)).await.is_err() {
                                                debug!("Receiver dropped");
                                                return;
                                            }

                                            last_sent_len = accumulated_text.len();

                                            if done {
                                                return;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        warn!("Failed to parse SSE event: {} - data length: {} bytes", e, data.len());

                                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                                            debug!("Valid JSON but wrong structure: {:?}", value);

                                            if data.contains("\"error\"") {
                                                let _ = tx
                                                    .send(Err(GeminiError::ApiError(format!(
                                                        "API error response: {value}"
                                                    ))))
                                                    .await;
                                                return;
                                            }
                                        } else {
                                            warn!("Invalid JSON - first 200 chars: {}",
                                                  if data.len() > 200 { &data[..200] } else { data });
                                        }

                                        let unsent_text = if accumulated_text.len() > last_sent_len {
                                            Some(accumulated_text[last_sent_len..].to_string())
                                        } else {
                                            None
                                        };

                                        if let Some(text) = &unsent_text {
                                            warn!("Malformed SSE event, salvaging {} chars of unsent text", text.len());
                                        } else {
                                            warn!("Malformed SSE event, no unsent text to salvage");
                                        }

                                        let salvage_response = StreamResponse {
                                            text: unsent_text,
                                            function_calls: accumulated_calls.clone(),
                                            done: true,
                                        };

                                        let _ = tx.send(Ok(salvage_response)).await;
                                        return;
                                    }
                                }
                                    event_data.clear();
                                }
                                continue;
                            }

                            if let Some(data) = line.strip_prefix("data: ") {
                                if !event_data.is_empty() {
                                    event_data.push('\n');
                                }
                                event_data.push_str(data);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("HTTP stream error: {} - salvaging {} chars of accumulated text", e, accumulated_text.len());

                        let unsent_text = if accumulated_text.len() > last_sent_len {
                            Some(accumulated_text[last_sent_len..].to_string())
                        } else {
                            None
                        };

                        if let Some(text) = &unsent_text {
                            debug!("Salvaging {} chars from interrupted stream", text.len());
                        }

                        // Send salvaged content instead of error
                        let salvage_response = StreamResponse {
                            text: unsent_text,
                            function_calls: accumulated_calls.clone(),
                            done: true,
                        };
                        let _ = tx.send(Ok(salvage_response)).await;
                        return;
                    }
                }
            }

            let unsent_text = if accumulated_text.len() > last_sent_len {
                Some(accumulated_text[last_sent_len..].to_string())
            } else {
                None
            };

            let final_response = StreamResponse {
                text: unsent_text,
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
