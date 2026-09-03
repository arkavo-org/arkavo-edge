use crate::Result;
use crate::decision::ModelChoice;
use arkavo_llm::ParsedToolCall;
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
        }
    }

    /// Get metadata about this routing request.
    ///
    /// This is available immediately, before streaming completes.
    pub fn metadata(&self) -> &RouteMetadata {
        &self.metadata
    }

    /// Tool calls this stream will report from `complete()`.
    pub fn tool_calls(&self) -> &[ParsedToolCall] {
        &self.tool_calls
    }

    /// Carry tool calls onto a stream rebuilt from another one.
    ///
    /// `new` starts with none, which is right for a stream being built from
    /// chunks and wrong for one that wraps a stream that already had them: a
    /// wrapper that forgets to carry them makes the tool calls disappear for
    /// the caller and only when the wrapper is in place.
    pub fn with_tool_calls(mut self, tool_calls: Vec<ParsedToolCall>) -> Self {
        self.tool_calls = tool_calls;
        self
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
            model: self.metadata.model,
            cost_usd: self.metadata.estimated_cost_usd,
            used_architect_mode: self.metadata.used_architect_mode,
            architect_savings: None,
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
            model: ModelChoice::LocalGemma270M,
            cost_usd: 0.0,
            used_architect_mode: false,
            architect_savings: None,
        };

        let stream = RouteStream::from_response(response);
        assert!(!stream.metadata().used_architect_mode);

        let result = stream.complete().await.unwrap();
        assert_eq!(result.content, "Hello, world!");
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
