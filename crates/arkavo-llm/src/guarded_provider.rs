//! Inspection at the provider boundary, before callers can execute or publish output.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;

use crate::{
    Error, GATE_BLOCKED, GateOutcome, Message, Provider, ProviderResponse, ReleaseGate, Result,
    StreamResponse,
};

/// Creates independent buffers for each completion using the serving model's policy.
#[async_trait]
pub trait ReleaseGateFactory: Send + Sync {
    fn create(&self, model: &str) -> Arc<dyn ReleaseGate>;
    /// Run response checks before the release policy. Implementations may
    /// contribute classifier evidence here without granting release themselves.
    async fn verify(&self, _response: &ProviderResponse) -> Result<()> {
        Ok(())
    }
}

pub struct GuardedProvider {
    inner: Box<dyn Provider>,
    factory: Arc<dyn ReleaseGateFactory>,
}

impl GuardedProvider {
    pub fn new(inner: Box<dyn Provider>, factory: Arc<dyn ReleaseGateFactory>) -> Self {
        Self { inner, factory }
    }

    async fn inspect(&self, response: ProviderResponse) -> Result<ProviderResponse> {
        self.factory.verify(&response).await?;
        let gate = self.factory.create(self.inner.name());
        tokio::task::spawn_blocking(move || {
            let mut text = response.content.clone();
            if let Some(reasoning) = &response.reasoning_content {
                text.push('\n');
                text.push_str(reasoning);
            }
            for call in &response.tool_calls {
                text.push('\n');
                text.push_str(&call.arguments.to_string());
            }
            let result = inspect_whole(&gate, &text);
            gate.discard();
            result.map(|()| response)
        })
        .await
        .map_err(|_| withheld())?
    }
}

fn withheld() -> Error {
    Error::Provider(GATE_BLOCKED.into())
}

fn inspect_whole(gate: &Arc<dyn ReleaseGate>, text: &str) -> Result<()> {
    if gate.admit(text) == GateOutcome::Blocked || gate.finish() == GateOutcome::Blocked {
        Err(withheld())
    } else {
        Ok(())
    }
}

#[async_trait]
impl Provider for GuardedProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        max_tokens: Option<usize>,
    ) -> Result<String> {
        let content = self
            .inner
            .complete_with_options(messages, max_tokens)
            .await?;
        Ok(self
            .inspect(ProviderResponse {
                content,
                ..Default::default()
            })
            .await?
            .content)
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<Message>,
        tools: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        let response = self
            .inner
            .complete_with_tools(messages, tools, max_tokens)
            .await?;
        self.inspect(response).await
    }

    async fn complete_with_schema(
        &self,
        messages: Vec<Message>,
        schema: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<String> {
        let content = self
            .inner
            .complete_with_schema(messages, schema, max_tokens)
            .await?;
        Ok(self
            .inspect(ProviderResponse {
                content,
                ..Default::default()
            })
            .await?
            .content)
    }

    #[allow(clippy::disallowed_methods)] // Dedicated blocking worker, never a Tokio executor thread.
    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn futures::Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let inner = self.inner.stream(messages).await?;
        let gate = self.factory.create(self.inner.name());
        let reasoning_gate = self.factory.create(self.inner.name());
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let runtime = tokio::runtime::Handle::current();
        let factory = self.factory.clone();
        // Blocking inspection never occupies a Tokio worker. The bounded channel
        // also bounds generation ahead of the consumer; disconnect cancels the read.
        tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
                let reasoning = Arc::new(std::sync::Mutex::new(String::new()));
                let captured = reasoning.clone();
                let inner = inner.map(move |item| {
                    item.map(|mut chunk| {
                        if let Some(text) = chunk.reasoning_content.take() {
                            captured
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .push_str(&text);
                        }
                        chunk
                    })
                });
                let mut stream = crate::gated(Box::pin(inner), gate);
                let mut completion = String::new();
                loop {
                    let item = tokio::select! {
                        _ = tx.closed() => break,
                        item = stream.next() => item,
                    };
                    let Some(mut item) = item else { break };
                    if let Ok(chunk) = &mut item {
                        completion.push_str(&chunk.content);
                    }
                    if let Ok(chunk) = &mut item
                        && chunk.done
                    {
                        let text = std::mem::take(
                            &mut *reasoning.lock().unwrap_or_else(|e| e.into_inner()),
                        );
                        let response = ProviderResponse {
                            content: completion.clone(),
                            reasoning_content: Some(text.clone()),
                            ..Default::default()
                        };
                        if factory.verify(&response).await.is_err()
                            || inspect_whole(&reasoning_gate, &text).is_err()
                        {
                            item = Err(withheld());
                        } else if !text.is_empty() {
                            chunk.reasoning_content = Some(text);
                        }
                    }
                    let failed = item.is_err();
                    if tx.send(item).await.is_err() || failed {
                        break;
                    }
                }
                reasoning_gate.discard();
            });
        });
        Ok(Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
    fn supports_tools(&self) -> bool {
        self.inner.supports_tools()
    }
    fn supports_structured_output(&self) -> bool {
        self.inner.supports_structured_output()
    }
}
