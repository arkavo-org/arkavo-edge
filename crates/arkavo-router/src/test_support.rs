//! Deterministic provider substitution for the crate's own tests.
//!
//! Every routing path resolves its provider through [`crate::ProviderFactory`],
//! so installing a [`CountingProvider`] on a `Router` removes credentials, the
//! model cache and the network from the test — and makes "this call was
//! refused before it reached a model" an assertion rather than an inference.
use crate::decision::ModelChoice;
use crate::error::Result;
use crate::provider::ProviderFactory;
use arkavo_llm::{Message, Provider, ProviderResponse};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(crate) struct CountingProvider {
    content: String,
    /// Emitted as a native tool call, for driving the validation-retry path.
    tool_call: Option<String>,
    calls: Arc<AtomicUsize>,
    builds: Arc<AtomicUsize>,
    /// Every model the router asked this factory to build, in order — so a
    /// test can assert *which* arm was instantiated, not just how many.
    built_models: Arc<Mutex<Vec<ModelChoice>>>,
    /// Zero-based index of the first dispatch that fails.
    fail_from: Option<usize>,
    /// Dispatches before this index answer with empty content, which the
    /// collapse detector reads as a breakdown.
    blank_before: usize,
}

impl CountingProvider {
    pub(crate) fn new(content: &str) -> Self {
        Self {
            content: content.to_string(),
            tool_call: None,
            calls: Arc::new(AtomicUsize::new(0)),
            builds: Arc::new(AtomicUsize::new(0)),
            built_models: Arc::new(Mutex::new(Vec::new())),
            fail_from: None,
            blank_before: 0,
        }
    }

    /// Collapse on the first dispatch, then answer normally.
    pub(crate) fn blank_then(content: &str) -> Self {
        Self {
            blank_before: 1,
            ..Self::new(content)
        }
    }

    pub(crate) fn failing_from(content: &str, call_index: usize) -> Self {
        Self {
            fail_from: Some(call_index),
            ..Self::new(content)
        }
    }

    /// Answer with a native call to `tool_name`, which an empty registry will
    /// reject — the deterministic way to make the quality gate retry.
    pub(crate) fn calling_tool(tool_name: &str) -> Self {
        Self {
            tool_call: Some(tool_name.to_string()),
            ..Self::new("")
        }
    }

    /// Dispatches that reached a model.
    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// Providers the router asked this factory to build.
    pub(crate) fn builds(&self) -> usize {
        self.builds.load(Ordering::SeqCst)
    }

    /// The models the router asked this factory to build, in order.
    pub(crate) fn built_models(&self) -> Vec<ModelChoice> {
        self.built_models.lock().expect("factory log").clone()
    }

    pub(crate) fn factory(&self) -> Arc<dyn ProviderFactory> {
        Arc::new(self.clone())
    }

    fn respond(&self) -> arkavo_llm::Result<ProviderResponse> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        let timing = arkavo_llm::InferenceTiming {
            n_prompt_eval: 1_000,
            n_eval: 10_000,
            ..Default::default()
        };
        if self.fail_from.is_some_and(|first| index >= first) {
            return Err(arkavo_llm::Error::ProviderResponseFailure {
                message: "substituted provider refused".into(),
                inference_timing: Some(timing),
            });
        }
        Ok(ProviderResponse {
            content: if index < self.blank_before {
                String::new()
            } else {
                self.content.clone()
            },
            tool_calls: self
                .tool_call
                .iter()
                .map(|name| arkavo_llm::tool_parser::ParsedToolCall {
                    tool_name: name.clone(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                })
                .collect(),
            inference_timing: Some(timing),
            ..Default::default()
        })
    }
}

#[async_trait::async_trait]
impl Provider for CountingProvider {
    async fn complete_with_options(
        &self,
        _: Vec<Message>,
        _: Option<usize>,
    ) -> arkavo_llm::Result<String> {
        self.respond().map(|response| response.content)
    }

    async fn stream(
        &self,
        _: Vec<Message>,
    ) -> arkavo_llm::Result<
        Box<
            dyn futures::Stream<Item = arkavo_llm::Result<arkavo_llm::StreamResponse>>
                + Send
                + Unpin,
        >,
    > {
        Ok(Box::new(futures::stream::empty()))
    }

    fn name(&self) -> &str {
        "substituted"
    }

    async fn complete_with_tools(
        &self,
        _: Vec<Message>,
        _: Option<serde_json::Value>,
        _: Option<usize>,
    ) -> arkavo_llm::Result<ProviderResponse> {
        self.respond()
    }

    async fn complete_with_schema_response(
        &self,
        _: Vec<Message>,
        _: Option<serde_json::Value>,
        _: Option<usize>,
    ) -> arkavo_llm::Result<ProviderResponse> {
        self.respond()
    }
}

impl ProviderFactory for CountingProvider {
    fn build(&self, model: &ModelChoice) -> Result<Box<dyn Provider>> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        self.built_models
            .lock()
            .expect("factory log")
            .push(model.clone());
        Ok(Box::new(self.clone()))
    }
}

/// Availability with exactly one cloud provider configured.
pub(crate) fn only(provider: &str) -> crate::ProviderAvailability {
    let mut availability = crate::ProviderAvailability::default();
    match provider {
        "openai" => availability.openai = true,
        "xai" => availability.xai = true,
        "gemini" => availability.gemini = true,
        "anthropic" => availability.anthropic = true,
        other => panic!("unsupported provider for test availability: {other}"),
    }
    availability
}

/// A router with no local weights, one configured cloud provider, a fixed
/// connectivity answer and every provider substituted.
pub(crate) async fn cloud_router(
    policy: arkavo_budget::CloudPolicy,
    provider_name: &str,
    provider: &CountingProvider,
) -> crate::Router {
    let mut router = crate::Router::new_offline().await.unwrap();
    router.set_offline_mode(false);
    router
        .with_cloud_policy(policy)
        .with_connectivity(crate::ConnectivityChecker::assume(true))
        .with_selector(crate::ModelSelector::with_availability(
            only(provider_name),
            false,
        ))
        .await
        .with_provider_factory(provider.factory())
}
