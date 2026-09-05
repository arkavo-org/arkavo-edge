//! Exercise the registered production provider boundary with controlled output.
#![cfg(feature = "sentinel")]
#![allow(clippy::disallowed_methods)] // Tokio test entrypoint owns its runtime.

use std::sync::Arc;
use std::time::Instant;

use arkavo_cli::sentinel_wiring::CascadeFactory;
use arkavo_fingerprint::{IndexKey, ReferenceIndex, ReferenceTier};
use arkavo_llm::{
    GATE_BLOCKED, Message, ParsedToolCall, Provider, ProviderResponse, Result, StreamResponse,
};
use arkavo_protocol::classification_evidence::{Confidence, LabelFinding, TierReport};
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use arkavo_router::response_policy;
use arkavo_sentinel::{Cascade, CascadeTier};
use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde_json::{Value, json};

const CANARY: &str = "the northwind acquisition closes in the third quarter pending board approval";
const CLEAN: &str = "Clear skies are expected throughout the coming week.";

struct PublicTier;

impl CascadeTier for PublicTier {
    fn name(&self) -> &'static str {
        "public-classifier"
    }

    fn examine_until(&self, text: &str, _deadline: Instant) -> TierReport {
        self.examine_unbudgeted(text)
    }

    fn examine_unbudgeted(&self, _text: &str) -> TierReport {
        TierReport::matched(
            self.name(),
            "1.0.0",
            vec![LabelFinding::new(
                DataCategory::Public,
                SensitivityLevel::Public,
                Confidence::new(1.0),
                "public",
            )],
        )
    }
}

fn cascade() -> Arc<Cascade> {
    let key = Arc::new(IndexKey::derive(&[17; 32], "runtime-regression").unwrap());
    let mut builder = ReferenceIndex::builder(&key, "1.0.0");
    builder.add_document(
        &key,
        CANARY,
        DataCategory::Internal,
        SensitivityLevel::Confidential,
        "board-minutes",
    );
    builder.add_document(
        &key,
        "cerulean permit status remains private",
        DataCategory::Internal,
        SensitivityLevel::Confidential,
        "split-field",
    );
    Arc::new(
        Cascade::new("1.0.0")
            .with_tier(Arc::new(ReferenceTier::loaded(
                Arc::new(builder.build()),
                key,
            )))
            .with_tier(Arc::new(PublicTier)),
    )
}

/// The provider controls only output; registration supplies all enforcement.
struct OutputProvider(ProviderResponse);

#[async_trait]
impl Provider for OutputProvider {
    async fn complete_with_options(&self, _: Vec<Message>, _: Option<usize>) -> Result<String> {
        Ok(self.0.content.clone())
    }

    async fn complete_with_tools(
        &self,
        _: Vec<Message>,
        _: Option<Value>,
        _: Option<usize>,
    ) -> Result<ProviderResponse> {
        Ok(self.0.clone())
    }

    async fn stream(
        &self,
        _: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        // Text on the done chunk must survive admission and final inspection.
        Ok(Box::new(futures::stream::iter(vec![Ok(StreamResponse {
            content: self.0.content.clone(),
            reasoning_content: self.0.reasoning_content.clone(),
            done: true,
            inference_timing: None,
        })])))
    }

    fn name(&self) -> &'static str {
        "runtime-regression"
    }
}

fn provider(response: ProviderResponse) -> Box<dyn Provider> {
    response_policy::protect(Box::new(OutputProvider(response)))
}

fn content(text: &str) -> ProviderResponse {
    ProviderResponse {
        content: text.into(),
        ..Default::default()
    }
}

fn assert_withheld<T: std::fmt::Debug>(result: Result<T>) {
    let error = result
        .expect_err("confidential output must be withheld")
        .to_string();
    assert!(error.contains(GATE_BLOCKED), "{error}");
    assert!(
        !error.contains("northwind"),
        "refusals must not disclose the finding"
    );
}

async fn assert_stream_withheld(provider: &dyn Provider) {
    let mut stream = provider.stream(vec![]).await.unwrap();
    let mut blocked = false;
    while let Some(item) = stream.next().await {
        match item {
            Ok(chunk) => {
                assert!(
                    chunk.content.is_empty(),
                    "no unverified completion is released"
                );
                assert!(chunk.reasoning_content.is_none());
            }
            Err(error) => {
                assert_withheld::<()>(Err(error));
                blocked = true;
            }
        }
    }
    assert!(blocked, "the stream must report the withheld response");
}

// A single test owns the process-wide immutable registration in this binary.
#[arkavo_test_macros::spec("SENT-007")]
#[tokio::test]
async fn registered_policy_protects_all_provider_output_paths() {
    response_policy::install(Arc::new(CascadeFactory::new(cascade()))).unwrap();
    assert!(response_policy::install(Arc::new(CascadeFactory::new(cascade()))).is_err());

    let confidential = provider(content(CANARY));
    assert_withheld(confidential.complete(vec![]).await);
    assert_withheld(confidential.complete_with_options(vec![], Some(64)).await);
    assert_withheld(
        confidential
            .complete_with_schema(vec![], Some(json!({"type": "string"})), None)
            .await,
    );
    assert_withheld(confidential.complete_with_tools(vec![], None, None).await);
    assert_stream_withheld(confidential.as_ref()).await;

    let tools = provider(ProviderResponse {
        tool_calls: vec![ParsedToolCall {
            tool_name: "send_message".into(),
            arguments: json!({"envelope": {"messages": [{"body": CANARY}]}}),
            call_id: Some("call-1".into()),
        }],
        ..content(CLEAN)
    });
    assert_withheld(tools.complete_with_tools(vec![], None, None).await);

    let reasoning = provider(ProviderResponse {
        reasoning_content: Some(CANARY.into()),
        ..content(CLEAN)
    });
    assert_withheld(reasoning.complete_with_tools(vec![], None, None).await);
    assert_stream_withheld(reasoning.as_ref()).await;

    // Classification is over the whole output, including field boundaries.
    let split = provider(ProviderResponse {
        reasoning_content: Some("status remains private".into()),
        ..content("cerulean permit")
    });
    assert_withheld(split.complete_with_tools(vec![], None, None).await);
    assert_stream_withheld(split.as_ref()).await;

    let public = provider(content(CLEAN));
    assert_eq!(public.complete(vec![]).await.unwrap(), CLEAN);
    assert_eq!(
        public
            .complete_with_schema(vec![], None, None)
            .await
            .unwrap(),
        CLEAN
    );
    assert_eq!(
        public
            .complete_with_tools(vec![], None, None)
            .await
            .unwrap()
            .content,
        CLEAN
    );
    let chunks: Vec<_> = public.stream(vec![]).await.unwrap().collect().await;
    let chunks: Vec<_> = chunks.into_iter().collect::<Result<_>>().unwrap();
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.content.as_str())
            .collect::<String>(),
        CLEAN
    );
    assert!(chunks.last().unwrap().done);
}
