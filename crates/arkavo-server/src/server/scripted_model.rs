//! A model whose replies a test writes in advance, behind a router that
//! dispatches every call to it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use arkavo_llm::{Message as Prompt, ParsedToolCall, Provider, ProviderResponse, StreamResponse};
use arkavo_router::{ModelChoice, ProviderFactory};

/// Replies handed out in order, counting the dispatches that asked for them
/// and noting whether each was offered tools and what it was shown.
#[derive(Default)]
pub(crate) struct Script {
    replies: Mutex<VecDeque<ProviderResponse>>,
    pub(crate) dispatches: Mutex<usize>,
    pub(crate) offered_tools: Mutex<Vec<bool>>,
    pub(crate) prompts: Mutex<Vec<String>>,
}

impl Script {
    pub(crate) fn of(replies: impl IntoIterator<Item = ProviderResponse>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().collect()),
            ..Self::default()
        })
    }
}

pub(crate) fn text(content: &str) -> ProviderResponse {
    ProviderResponse {
        content: content.to_string(),
        ..Default::default()
    }
}

pub(crate) fn calling(tool_name: &str) -> ProviderResponse {
    calling_all(&[tool_name])
}

/// One round that calls every tool in `tool_names`, in that order.
pub(crate) fn calling_all(tool_names: &[&str]) -> ProviderResponse {
    ProviderResponse {
        tool_calls: tool_names
            .iter()
            .map(|tool_name| ParsedToolCall {
                tool_name: (*tool_name).to_string(),
                arguments: serde_json::json!({}),
                call_id: None,
            })
            .collect(),
        ..Default::default()
    }
}

struct ScriptedProvider(Arc<Script>);

#[async_trait::async_trait]
impl Provider for ScriptedProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Prompt>,
        max_tokens: Option<usize>,
    ) -> arkavo_llm::Result<String> {
        self.complete_with_tools(messages, None, max_tokens)
            .await
            .map(|reply| reply.content)
    }

    async fn stream(
        &self,
        _messages: Vec<Prompt>,
    ) -> arkavo_llm::Result<
        Box<dyn futures::Stream<Item = arkavo_llm::Result<StreamResponse>> + Send + Unpin>,
    > {
        Ok(Box::new(futures::stream::empty()))
    }

    fn name(&self) -> &'static str {
        "scripted"
    }

    async fn complete_with_tools(
        &self,
        messages: Vec<Prompt>,
        tools: Option<serde_json::Value>,
        _max_tokens: Option<usize>,
    ) -> arkavo_llm::Result<ProviderResponse> {
        *self.0.dispatches.lock().expect("dispatch count") += 1;
        let shown: Vec<_> = messages.into_iter().map(|m| m.content).collect();
        self.0
            .prompts
            .lock()
            .expect("prompts")
            .push(shown.join("\n"));
        self.0
            .offered_tools
            .lock()
            .expect("tool offers")
            .push(tools.is_some());
        Ok(self
            .0
            .replies
            .lock()
            .expect("reply queue")
            .pop_front()
            .unwrap_or_default())
    }
}

struct ScriptedFactory(Arc<Script>);

impl ProviderFactory for ScriptedFactory {
    fn build(&self, _model: &ModelChoice) -> arkavo_router::Result<Box<dyn Provider>> {
        Ok(Box::new(ScriptedProvider(self.0.clone())))
    }
}

pub(crate) async fn scripted_router(script: &Arc<Script>) -> Arc<arkavo_router::Router> {
    let availability = arkavo_router::ProviderAvailability {
        xai: true,
        ..Default::default()
    };
    let mut router = arkavo_router::Router::new_offline().await.expect("router");
    router.set_offline_mode(false);
    Arc::new(
        router
            .with_cloud_policy(arkavo_budget::CloudPolicy::AskBeforeCloud)
            .with_connectivity(arkavo_router::ConnectivityChecker::assume(true))
            .with_selector(arkavo_router::ModelSelector::with_availability(
                availability,
                false,
            ))
            .await
            .with_provider_factory(Arc::new(ScriptedFactory(script.clone()))),
    )
}
