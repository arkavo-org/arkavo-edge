#![allow(clippy::disallowed_methods)]
#![allow(clippy::significant_drop_tightening)]
#![allow(unexpected_cfgs)]
#![allow(unused_variables)]
#![allow(clippy::float_cmp)]
#![allow(clippy::redundant_pub_crate)]
#![allow(clippy::needless_collect)]

use arkavo_llm::{Message, Provider, StreamResponse};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_stream::Stream;

/// Simple mock provider for testing
pub(crate) struct MockProvider {
    responses: Arc<RwLock<Vec<String>>>,
    current_index: Arc<RwLock<usize>>,
    pub request_count: Arc<RwLock<usize>>,
}

impl MockProvider {
    pub(crate) fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Arc::new(RwLock::new(responses)),
            current_index: Arc::new(RwLock::new(0)),
            request_count: Arc::new(RwLock::new(0)),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn with_single_response(response: String) -> Self {
        Self::new(vec![response])
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn complete(&self, _messages: Vec<Message>) -> Result<String, arkavo_llm::Error> {
        self.complete_with_options(_messages, None).await
    }

    async fn complete_with_options(
        &self,
        _messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> Result<String, arkavo_llm::Error> {
        *self.request_count.write().await += 1;

        let responses = self.responses.read().await;
        let mut index = self.current_index.write().await;

        if responses.is_empty() {
            return Err(arkavo_llm::Error::Provider(
                "No responses configured".to_string(),
            ));
        }

        let response = responses[*index % responses.len()].clone();
        *index += 1;

        Ok(response)
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<
        Box<dyn Stream<Item = Result<StreamResponse, arkavo_llm::Error>> + Send + Unpin>,
        arkavo_llm::Error,
    > {
        let response = self.complete(messages).await?;

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            let _ = tx.send(Ok(StreamResponse {
                content: response,
                reasoning_content: None,
                done: true,
            }));
        });

        Ok(Box::new(
            tokio_stream::wrappers::UnboundedReceiverStream::new(rx),
        ))
    }

    fn name(&self) -> &'static str {
        "mock"
    }
}
