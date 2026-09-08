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
        let timing = response.inference_timing.clone();
        self.factory
            .verify(&response)
            .await
            .map_err(|e| e.with_inference_timing(timing.clone()))?;
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
        .map_err(|_| withheld().with_inference_timing(timing.clone()))?
        .map_err(|e| e.with_inference_timing(timing))
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
        let response = self
            .inner
            .complete_with_tools(messages, None, max_tokens)
            .await?;
        Ok(self.inspect(response).await?.content)
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
        let response = self
            .inner
            .complete_with_schema_response(messages, schema, max_tokens)
            .await?;
        Ok(self.inspect(response).await?.content)
    }

    async fn complete_with_schema_response(
        &self,
        messages: Vec<Message>,
        schema: Option<Value>,
        max_tokens: Option<usize>,
    ) -> Result<ProviderResponse> {
        let response = self
            .inner
            .complete_with_schema_response(messages, schema, max_tokens)
            .await?;
        self.inspect(response).await
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
                let timing = Arc::new(std::sync::Mutex::new(None));
                let captured_timing = timing.clone();
                let inner = inner.map(move |item| {
                    item.map(|mut chunk| {
                        if chunk.inference_timing.is_some() {
                            captured_timing
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .clone_from(&chunk.inference_timing);
                        }
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
                            || inspect_whole(&reasoning_gate, &format!("{completion}\n{text}"))
                                .is_err()
                        {
                            item = Err(withheld());
                        } else if !text.is_empty() {
                            chunk.reasoning_content = Some(text);
                        }
                    }
                    item = item.map_err(|e| {
                        e.with_inference_timing(
                            timing.lock().unwrap_or_else(|e| e.into_inner()).clone(),
                        )
                    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_parser::ParsedToolCall;
    use arkavo_test_macros::spec;
    use std::sync::Mutex;

    /// Blocks any completion whose inspected text names the canary.
    struct BlockCanary;

    impl ReleaseGate for BlockCanary {
        fn admit(&self, chunk: &str) -> GateOutcome {
            if chunk.contains("canary") {
                GateOutcome::Blocked
            } else {
                GateOutcome::Release(chunk.to_string())
            }
        }
        fn finish(&self) -> GateOutcome {
            GateOutcome::Release(String::new())
        }
        fn discard(&self) {}
    }

    struct Policy;

    impl ReleaseGateFactory for Policy {
        fn create(&self, _model: &str) -> Arc<dyn ReleaseGate> {
            Arc::new(BlockCanary)
        }
    }

    /// A provider with native tools: its `complete_with_tools` is the only path
    /// that reports reasoning and tool calls.
    struct NativeTools {
        seen: Arc<Mutex<Vec<(bool, Option<usize>)>>>,
    }

    #[async_trait]
    impl Provider for NativeTools {
        async fn complete_with_options(
            &self,
            _messages: Vec<Message>,
            _max_tokens: Option<usize>,
        ) -> Result<String> {
            Ok("text-only path".into())
        }
        async fn complete_with_tools(
            &self,
            _messages: Vec<Message>,
            tools: Option<Value>,
            max_tokens: Option<usize>,
        ) -> Result<ProviderResponse> {
            self.seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((tools.is_none(), max_tokens));
            Ok(ProviderResponse {
                content: "the visible answer".into(),
                reasoning_content: Some("a canary in the reasoning".into()),
                tool_calls: vec![ParsedToolCall {
                    tool_name: "read".into(),
                    arguments: serde_json::json!({}),
                    call_id: Some("call_1".into()),
                }],
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            _messages: Vec<Message>,
        ) -> Result<Box<dyn futures::Stream<Item = Result<StreamResponse>> + Send + Unpin>>
        {
            unimplemented!("this fixture never streams")
        }
        fn name(&self) -> &str {
            "native-tools"
        }
        fn supports_tools(&self) -> bool {
            true
        }
    }

    /// A provider without native tools: it implements only the text path and
    /// inherits the trait's default `complete_with_tools`.
    struct TextOnly;

    #[async_trait]
    impl Provider for TextOnly {
        async fn complete_with_options(
            &self,
            _messages: Vec<Message>,
            _max_tokens: Option<usize>,
        ) -> Result<String> {
            Ok("plain completion".into())
        }
        async fn stream(
            &self,
            _messages: Vec<Message>,
        ) -> Result<Box<dyn futures::Stream<Item = Result<StreamResponse>> + Send + Unpin>>
        {
            unimplemented!("this fixture never streams")
        }
        fn name(&self) -> &str {
            "text-only"
        }
    }

    /// Text completions route through `complete_with_tools` so the gate sees the
    /// whole turn — reasoning included. Reaching for `complete_with_options`
    /// instead would hand the caller text whose reasoning was never inspected.
    ///
    /// The dispatch also means a provider that answers the two paths
    /// differently answers on the tool path here: on Anthropic that honours
    /// `max_tokens` (its text path ignores it) and forgoes that path's retries.
    #[spec("SENT-007")]
    #[tokio::test]
    async fn a_text_completion_is_gated_on_the_whole_turn() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let guarded = GuardedProvider::new(
            Box::new(NativeTools {
                seen: Arc::clone(&seen),
            }),
            Arc::new(Policy) as Arc<dyn ReleaseGateFactory>,
        );

        let error = guarded
            .complete_with_options(vec![Message::user("hello")], Some(256))
            .await
            .expect_err("reasoning naming the canary must not be released as clean text");
        assert!(error.to_string().contains(GATE_BLOCKED), "{error}");
        // The wrapped provider was asked for a completion, not for tool use,
        // and the caller's ceiling reached it.
        assert_eq!(
            *seen.lock().unwrap_or_else(|e| e.into_inner()),
            vec![(true, Some(256))]
        );
    }

    /// A provider without native tools has no separate tool path, so the
    /// dispatch reaches its text completion through the trait's default.
    #[spec("SENT-007")]
    #[tokio::test]
    async fn a_provider_without_native_tools_still_completes() {
        let guarded = GuardedProvider::new(
            Box::new(TextOnly),
            Arc::new(Policy) as Arc<dyn ReleaseGateFactory>,
        );
        let content = guarded
            .complete_with_options(vec![Message::user("hello")], None)
            .await
            .unwrap();
        assert_eq!(content, "plain completion");
    }
}
