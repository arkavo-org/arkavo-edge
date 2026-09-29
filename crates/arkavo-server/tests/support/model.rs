//! A scripted model for tests that must not load one.
//!
//! The router resolves every dispatch through an installed
//! `ProviderFactory`, so a script of replies stands in for the model and the
//! test can assert on what the model was asked and how often.
//!
//! Kept apart from the agent loop harness so a test binary that drives the
//! conductor directly can include this file alone.

// `pub(crate)` here trips clippy's `redundant_pub_crate` and plain `pub` trips
// rustc's `unreachable_pub`; the test crates in this repo settle it the same
// way, as a module shared by test binaries is never reachable outside.
#![allow(unreachable_pub)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use arkavo_llm::{Message, Provider, ProviderResponse, StreamResponse};
use arkavo_router::{ModelChoice, ProviderFactory};

/// Replies handed out in order, and a record of every prompt that asked.
#[derive(Default)]
pub struct Script {
    replies: Mutex<VecDeque<ProviderResponse>>,
    prompts: Mutex<Vec<Vec<Message>>>,
}

impl Script {
    pub fn new(replies: Vec<ProviderResponse>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            prompts: Mutex::new(Vec::new()),
        })
    }

    /// Dispatches that reached the model.
    pub fn dispatches(&self) -> usize {
        self.prompts.lock().expect("prompt log").len()
    }

    /// The conversation the model saw on its `index`th dispatch, as one string.
    pub fn prompt(&self, index: usize) -> String {
        self.prompts.lock().expect("prompt log")[index]
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn reply(&self, messages: Vec<Message>) -> ProviderResponse {
        self.prompts.lock().expect("prompt log").push(messages);
        self.replies
            .lock()
            .expect("reply queue")
            .pop_front()
            .unwrap_or_else(|| text("reply beyond the end of the script"))
    }
}

/// A reply that answers in prose and calls no tool.
pub fn text(content: &str) -> ProviderResponse {
    ProviderResponse {
        content: content.to_string(),
        ..Default::default()
    }
}

struct ScriptedProvider(Arc<Script>);

#[async_trait::async_trait]
impl Provider for ScriptedProvider {
    async fn complete_with_options(
        &self,
        messages: Vec<Message>,
        _max_tokens: Option<usize>,
    ) -> arkavo_llm::Result<String> {
        Ok(self.0.reply(messages).content)
    }

    async fn stream(
        &self,
        _messages: Vec<Message>,
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
        messages: Vec<Message>,
        _tools: Option<serde_json::Value>,
        _max_tokens: Option<usize>,
    ) -> arkavo_llm::Result<ProviderResponse> {
        Ok(self.0.reply(messages))
    }
}

struct ScriptedFactory(Arc<Script>);

impl ProviderFactory for ScriptedFactory {
    fn build(&self, _model: &ModelChoice) -> arkavo_router::Result<Box<dyn Provider>> {
        Ok(Box::new(ScriptedProvider(self.0.clone())))
    }
}

/// The model every scripted agent names, so routing is an explicit choice and
/// never consults the host's model cache.
pub const SCRIPTED_MODEL: ModelChoice = ModelChoice::Grok47;

/// A router whose only arm is the script.
pub async fn scripted_router(script: &Arc<Script>) -> Arc<arkavo_router::Router> {
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
