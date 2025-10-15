use crate::{ChatRequest, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use uuid::Uuid;

/// Unique identifier for a stream
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamId(pub String);

impl StreamId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }
}

impl Default for StreamId {
    fn default() -> Self {
        Self::new()
    }
}

/// Reason for stream ending
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Complete,
    MaxTokens,
    Aborted,
    Error(String),
    Timeout,
}

/// Type of error that occurred during streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

/// Typed delta events for streaming responses
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeltaType {
    /// Text content to append
    Text { content: String },

    /// Tool call being constructed
    ToolCall {
        id: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        arguments: Option<Value>,
    },

    /// Error occurred during streaming
    Error(StreamError),

    /// Stream has ended
    StreamEnd { reason: EndReason },
}

/// A single delta in the stream
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamDelta {
    pub stream_id: StreamId,
    pub sequence: u64,
    pub delta: DeltaType,
    pub trace_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Stream of deltas with back-pressure support
pub type DeltaStream = Pin<Box<dyn Stream<Item = Result<StreamDelta>> + Send>>;

/// Control commands for stream management
#[derive(Debug, Clone)]
pub enum StreamControl {
    Pause,
    Resume,
    Abort,
}

/// Abstraction for LLM providers that support streaming
#[async_trait]
pub trait StreamLlmModel: Send + Sync {
    /// Start a streaming chat session
    async fn stream_chat(
        &self,
        request: ChatRequest,
        trace_id: String,
    ) -> Result<(StreamId, DeltaStream)>;

    /// Send control command to an active stream
    async fn control_stream(&self, stream_id: &StreamId, command: StreamControl) -> Result<()>;

    /// Check if this provider supports pause/resume
    fn supports_pause(&self) -> bool {
        false
    }

    /// Get provider-specific metadata
    fn metadata(&self) -> Value {
        serde_json::json!({})
    }
}

/// Helper to create a bounded delta stream with back-pressure
pub fn create_bounded_stream(
    buffer_size: usize,
) -> (mpsc::Sender<Result<StreamDelta>>, DeltaStream) {
    let (tx, rx) = mpsc::channel(buffer_size);
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    (tx, Box::pin(stream))
}

/// Convert simple text chunks into typed deltas
pub async fn text_to_delta_stream<S>(
    source: S,
    stream_id: StreamId,
    trace_id: String,
) -> DeltaStream
where
    S: Stream<Item = Result<String>> + Send + 'static,
{
    use futures::StreamExt;

    let mut sequence = 0u64;
    let stream = source.map(move |result| {
        let current_sequence = sequence;
        sequence += 1;
        match result {
            Ok(text) => Ok(StreamDelta {
                stream_id: stream_id.clone(),
                sequence: current_sequence,
                delta: DeltaType::Text { content: text },
                trace_id: trace_id.clone(),
                timestamp: chrono::Utc::now(),
            }),
            Err(e) => Ok(StreamDelta {
                stream_id: stream_id.clone(),
                sequence: current_sequence,
                delta: DeltaType::Error(StreamError {
                    code: "STREAM_ERROR".to_string(),
                    message: e.to_string(),
                    retryable: false,
                }),
                trace_id: trace_id.clone(),
                timestamp: chrono::Utc::now(),
            }),
        }
    });

    Box::pin(stream)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_text_to_delta_stream() {
        let chunks = vec![
            Ok("Hello".to_string()),
            Ok(" world".to_string()),
            Err(crate::Error::Stream("Connection lost".to_string())),
        ];

        let source = tokio_stream::iter(chunks);
        let stream_id = StreamId::new();
        let trace_id = "test-trace".to_string();

        let mut delta_stream =
            text_to_delta_stream(source, stream_id.clone(), trace_id.clone()).await;

        // First delta
        let delta1 = delta_stream.next().await.unwrap().unwrap();
        assert_eq!(delta1.sequence, 0);
        assert_eq!(delta1.stream_id, stream_id);
        assert_eq!(delta1.trace_id, trace_id);
        match delta1.delta {
            DeltaType::Text { content } => assert_eq!(content, "Hello"),
            _ => panic!("Expected text delta"),
        }

        // Second delta
        let delta2 = delta_stream.next().await.unwrap().unwrap();
        assert_eq!(delta2.sequence, 1);
        match delta2.delta {
            DeltaType::Text { content } => assert_eq!(content, " world"),
            _ => panic!("Expected text delta"),
        }

        // Error delta
        let delta3 = delta_stream.next().await.unwrap().unwrap();
        assert_eq!(delta3.sequence, 2); // Sequence increments for each item
        match delta3.delta {
            DeltaType::Error(err) => {
                assert_eq!(err.code, "STREAM_ERROR");
                assert!(err.message.contains("Connection lost"));
            }
            _ => panic!("Expected error delta"),
        }
    }
}
