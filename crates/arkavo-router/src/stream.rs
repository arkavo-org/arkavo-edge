use crate::Result;
use crate::decision::ModelChoice;
use arkavo_llm::{InferenceTiming, ParsedToolCall, ProviderState};
use futures::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A stream of routing response chunks that can be iterated or awaited.
///
/// This is the unified return type for all routing operations. Callers can:
/// - **Stream**: iterate chunks for real-time UI display
/// - **Await**: call `.complete()` to get the final `RouteResponse`
pub struct RouteStream {
    inner: Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>,
    metadata: RouteMetadata,
    accumulated: String,
    /// Tool calls from the response (populated when created from_response)
    tool_calls: Vec<ParsedToolCall>,
    provider_state: ProviderState,
    reasoning_content: Option<String>,
    inference_timing: Option<InferenceTiming>,
    architect_savings: Option<f64>,
}

/// A single chunk in the response stream.
#[derive(Debug, Clone)]
pub struct StreamChunk {
    /// The content of this chunk
    pub content: String,
    /// Whether this is the final chunk
    pub done: bool,
}

/// Metadata available before streaming completes.
#[derive(Debug, Clone)]
pub struct RouteMetadata {
    /// The model being used for this request
    pub model: ModelChoice,
    /// Whether architect mode was activated for this request
    pub used_architect_mode: bool,
    /// Estimated cost in USD (may be refined after completion)
    pub estimated_cost_usd: f64,
}

/// The final response after streaming completes.
#[derive(Debug, Clone)]
pub struct RouteResponse {
    /// The complete response content
    pub content: String,
    /// Tool calls requested by the model (if any)
    pub tool_calls: Vec<ParsedToolCall>,
    /// Opaque provider state needed for stateless tool continuations.
    pub provider_state: ProviderState,
    pub reasoning_content: Option<String>,
    pub inference_timing: Option<InferenceTiming>,
    /// The model that produced this response
    pub model: ModelChoice,
    /// Actual cost in USD
    pub cost_usd: f64,
    /// Whether architect mode was used
    pub used_architect_mode: bool,
    /// Savings from architect mode (if used), in USD
    pub architect_savings: Option<f64>,
}

impl RouteStream {
    /// Create a new RouteStream from an inner stream and metadata.
    pub fn new(
        inner: Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>,
        metadata: RouteMetadata,
    ) -> Self {
        Self {
            inner,
            metadata,
            accumulated: String::new(),
            tool_calls: Vec::new(),
            provider_state: ProviderState::default(),
            reasoning_content: None,
            inference_timing: None,
            architect_savings: None,
        }
    }

    /// Create a RouteStream from a completed response (no streaming).
    pub fn from_response(response: RouteResponse) -> Self {
        let content = response.content.clone();
        let tool_calls = response.tool_calls.clone();
        let metadata = RouteMetadata {
            model: response.model.clone(),
            used_architect_mode: response.used_architect_mode,
            estimated_cost_usd: response.cost_usd,
        };

        let chunk = StreamChunk {
            content,
            done: true,
        };

        let stream = futures::stream::once(async move { Ok(chunk) });

        Self {
            inner: Box::pin(stream),
            metadata,
            accumulated: String::new(),
            tool_calls,
            provider_state: response.provider_state,
            reasoning_content: response.reasoning_content,
            inference_timing: response.inference_timing,
            architect_savings: response.architect_savings,
        }
    }

    /// Get metadata about this routing request.
    ///
    /// This is available immediately, before streaming completes.
    pub fn metadata(&self) -> &RouteMetadata {
        &self.metadata
    }

    /// Await the complete response, consuming the stream.
    ///
    /// This collects all chunks and returns the final `RouteResponse`.
    pub async fn complete(mut self) -> Result<RouteResponse> {
        use futures::StreamExt;

        while let Some(chunk_result) = self.inner.next().await {
            let chunk = chunk_result?;
            self.accumulated.push_str(&chunk.content);
        }

        Ok(RouteResponse {
            content: self.accumulated,
            tool_calls: self.tool_calls,
            provider_state: self.provider_state,
            reasoning_content: self.reasoning_content,
            inference_timing: self.inference_timing,
            model: self.metadata.model,
            cost_usd: self.metadata.estimated_cost_usd,
            used_architect_mode: self.metadata.used_architect_mode,
            architect_savings: self.architect_savings,
        })
    }
}

impl Stream for RouteStream {
    type Item = Result<StreamChunk>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("ROUTER-005")]
    #[tokio::test]
    async fn test_route_stream_from_response() {
        let response = RouteResponse {
            content: "Hello, world!".to_string(),
            tool_calls: vec![],
            provider_state: ProviderState::openai_responses(vec![
                serde_json::json!({"type":"reasoning", "encrypted_content":"opaque"}),
            ]),
            reasoning_content: Some("Summary".into()),
            inference_timing: None,
            model: ModelChoice::LocalGemma270M,
            cost_usd: 0.0,
            used_architect_mode: false,
            architect_savings: None,
        };

        let stream = RouteStream::from_response(response);
        assert!(!stream.metadata().used_architect_mode);

        let result = stream.complete().await.unwrap();
        assert_eq!(result.content, "Hello, world!");
        let items = result
            .provider_state
            .replay_items_for(arkavo_llm::ProviderStateTag::OpenAiResponses)
            .expect("openai state is replayable");
        assert_eq!(items[0]["encrypted_content"], "opaque");
        assert_eq!(result.reasoning_content.as_deref(), Some("Summary"));
    }

    /// Backpressure: a bounded channel with capacity 100 blocks the producer
    /// when the consumer cannot keep up, so the buffer never grows without bound.
    /// This exercises ROUTER-005 from the slow-consumer angle.
    #[spec("ROUTER-005")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_bounded_stream_backpressure_blocks_sender() {
        use futures::StreamExt;
        use std::time::Duration;

        // tokio::sync::mpsc::channel buffers exactly `capacity` messages; the
        // 101st send on a capacity-100 channel blocks until the consumer makes
        // room. This matches ROUTER-005's "buffer size capped at 100 chunks".
        const BUFFER: usize = 100;
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamChunk>>(BUFFER);
        let metadata = RouteMetadata {
            model: ModelChoice::LocalGemma270M,
            used_architect_mode: false,
            estimated_cost_usd: 0.0,
        };
        let mut stream = RouteStream::new(
            Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)),
            metadata,
        );

        // Fill the buffer; each send succeeds while capacity remains.
        for i in 0..BUFFER {
            tx.send(Ok(StreamChunk {
                content: format!("chunk-{i}"),
                done: false,
            }))
            .await
            .unwrap();
        }

        // The next send should be rejected immediately because the buffer is full.
        assert!(
            matches!(
                tx.try_send(Ok(StreamChunk {
                    content: "overflow-chunk".to_string(),
                    done: false,
                })),
                Err(tokio::sync::mpsc::error::TrySendError::Full(_))
            ),
            "bounded channel should apply backpressure when buffer is full"
        );

        // Start a slow consumer that drains everything after a delay, then send
        // the overflow chunk. This proves the producer can make progress once
        // the consumer creates capacity.
        let consumer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut count = 0;
            while let Some(Ok(chunk)) = stream.next().await {
                assert!(!chunk.done);
                count += 1;
                if chunk.content == "overflow-chunk" {
                    break;
                }
            }
            count
        });

        tx.send(Ok(StreamChunk {
            content: "overflow-chunk".to_string(),
            done: false,
        }))
        .await
        .unwrap();

        let drained = consumer.await.expect("consumer task finished");
        assert_eq!(drained, BUFFER + 1);
    }
}
