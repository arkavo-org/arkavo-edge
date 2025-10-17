use crate::{
    ChatRequest, DeltaStream, DeltaType, EndReason, LlmClient, Result, StreamControl, StreamDelta,
    StreamId, StreamLlmModel, create_bounded_stream,
};
use async_trait::async_trait;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, oneshot};

/// Adapter to convert LlmClient into StreamLlmModel
pub struct LlmClientAdapter {
    client: Arc<LlmClient>,
    active_streams: Arc<RwLock<HashMap<StreamId, oneshot::Sender<StreamControl>>>>,
}

impl LlmClientAdapter {
    pub fn new(client: LlmClient) -> Self {
        Self {
            client: Arc::new(client),
            active_streams: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl StreamLlmModel for LlmClientAdapter {
    async fn stream_chat(
        &self,
        request: ChatRequest,
        trace_id: String,
    ) -> Result<(StreamId, DeltaStream)> {
        let stream_id = StreamId::new();
        let (control_tx, control_rx) = oneshot::channel();

        // Store control channel
        self.active_streams
            .write()
            .await
            .insert(stream_id.clone(), control_tx);

        // Get the underlying stream
        let response_stream = self.client.chat_stream(request).await?;

        // Create bounded output stream
        let (delta_tx, delta_stream) = create_bounded_stream(32);

        // Spawn conversion task
        let stream_id_clone = stream_id.clone();
        let active_streams = self.active_streams.clone();
        let trace_id_clone = trace_id.clone();

        tokio::spawn(async move {
            let mut sequence = 0u64;
            let mut control_rx = control_rx;
            let mut response_stream = response_stream;
            let mut is_paused = false;

            loop {
                tokio::select! {
                    // Handle control commands
                    control = &mut control_rx => {
                        match control {
                            Ok(StreamControl::Abort) => {
                                // Send abort delta
                                let abort_delta = StreamDelta {
                                    stream_id: stream_id_clone.clone(),
                                    sequence,
                                    delta: DeltaType::StreamEnd { reason: EndReason::Aborted },
                                    trace_id: trace_id_clone.clone(),
                                    timestamp: chrono::Utc::now(),
                                };
                                let _ = delta_tx.send(Ok(abort_delta)).await;
                                break;
                            }
                            Ok(StreamControl::Pause) => {
                                is_paused = true;
                            }
                            Ok(StreamControl::Resume) => {
                                is_paused = false;
                            }
                            Err(_) => {
                                // Control channel closed, continue streaming
                            }
                        }
                    }

                    // Handle response chunks (if not paused)
                    Some(chunk_result) = response_stream.next(), if !is_paused => {
                        match chunk_result {
                            Ok(response) => {
                                let delta = StreamDelta {
                                    stream_id: stream_id_clone.clone(),
                                    sequence,
                                    delta: DeltaType::Text { content: response.content },
                                    trace_id: trace_id_clone.clone(),
                                    timestamp: chrono::Utc::now(),
                                };
                                sequence += 1;

                                if delta_tx.send(Ok(delta)).await.is_err() {
                                    // Receiver dropped
                                    break;
                                }

                                if response.done {
                                    // Send completion delta
                                    let end_delta = StreamDelta {
                                        stream_id: stream_id_clone.clone(),
                                        sequence,
                                        delta: DeltaType::StreamEnd { reason: EndReason::Complete },
                                        trace_id: trace_id_clone.clone(),
                                        timestamp: chrono::Utc::now(),
                                    };
                                    let _ = delta_tx.send(Ok(end_delta)).await;
                                    break;
                                }
                            }
                            Err(e) => {
                                // Send error delta
                                let error_delta = StreamDelta {
                                    stream_id: stream_id_clone.clone(),
                                    sequence,
                                    delta: DeltaType::Error(crate::StreamError {
                                        code: "LLM_ERROR".to_string(),
                                        message: e.to_string(),
                                        retryable: true,
                                    }),
                                    trace_id: trace_id_clone.clone(),
                                    timestamp: chrono::Utc::now(),
                                };
                                let _ = delta_tx.send(Ok(error_delta)).await;

                                // Send end delta
                                sequence += 1;
                                let end_delta = StreamDelta {
                                    stream_id: stream_id_clone.clone(),
                                    sequence,
                                    delta: DeltaType::StreamEnd { reason: EndReason::Error(e.to_string()) },
                                    trace_id: trace_id_clone.clone(),
                                    timestamp: chrono::Utc::now(),
                                };
                                let _ = delta_tx.send(Ok(end_delta)).await;
                                break;
                            }
                        }
                    }

                    else => {
                        // Stream ended naturally or paused indefinitely
                        if !is_paused {
                            break;
                        }
                        // If paused, wait a bit before checking again
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }

            // Clean up
            active_streams.write().await.remove(&stream_id_clone);
        });

        Ok((stream_id, delta_stream))
    }

    async fn control_stream(&self, stream_id: &StreamId, command: StreamControl) -> Result<()> {
        let mut streams = self.active_streams.write().await;
        if let Some(control_tx) = streams.remove(stream_id) {
            control_tx
                .send(command)
                .map_err(|_| crate::Error::Stream("Failed to send control command".to_string()))?;
            Ok(())
        } else {
            Err(crate::Error::Stream("Stream not found".to_string()))
        }
    }

    fn supports_pause(&self) -> bool {
        true // We implement pause/resume in the adapter
    }

    fn metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "provider": self.client.provider_name(),
            "adapter": "LlmClientAdapter",
            "supports_pause": true,
        })
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::LlmClient;

    #[tokio::test]
    async fn test_adapter_creation() {
        // This test requires a real LLM client, so we'll skip in CI
        if std::env::var("LLM_PROVIDER").is_err() {
            return;
        }

        let client = LlmClient::from_env().unwrap();
        let adapter = LlmClientAdapter::new(client);

        assert!(adapter.supports_pause());
        let metadata = adapter.metadata();
        assert_eq!(metadata["adapter"], "LlmClientAdapter");
    }
}
