//! What a requester gets back from `message/send` when no agent loop is
//! running and the handler executes the task itself.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arkavo_crypto::AgentKeypair;
use arkavo_gossip::{GossipConfig, GossipMessage};
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_llm::{Message as Prompt, ParsedToolCall, Provider, ProviderResponse, StreamResponse};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::MetricsCollector;
use arkavo_protocol::rate_limit::{RateLimitConfig, RateLimiter};
use arkavo_protocol::types::{Message, MessagePart, MessageSendRequest};
use arkavo_router::{ModelChoice, ProviderFactory};
use arkavo_tasks::task_executor::{TaskExecutor, TaskExecutorConfig};
use arkavo_tasks::task_store::{SqliteTaskStore, TaskStore};

use super::handle_message_send;
use crate::server::LearningBus;
use crate::server::config_helpers::AgentMetadata;
use crate::server::tool_memory::ToolMemory;

/// Replies handed out in order, counting the dispatches that asked for them.
#[derive(Default)]
struct Script {
    replies: Mutex<VecDeque<ProviderResponse>>,
    dispatches: Mutex<usize>,
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
        _messages: Vec<Prompt>,
        _tools: Option<serde_json::Value>,
        _max_tokens: Option<usize>,
    ) -> arkavo_llm::Result<ProviderResponse> {
        *self.0.dispatches.lock().expect("dispatch count") += 1;
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

async fn scripted_router(script: &Arc<Script>) -> Arc<arkavo_router::Router> {
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

/// Regression: the planner answered a text reply with "You MUST use a tool
/// now", so the requester was handed the output of `list_agents` instead of
/// the answer. This is the same fault on the path that runs without an agent
/// loop.
#[tokio::test]
async fn a_text_answer_is_the_result_of_the_task() {
    let answer = "Organic search drives 62% of signups; paid social converts worst.";
    let script = Arc::new(Script {
        replies: Mutex::new(VecDeque::from([
            ProviderResponse {
                content: answer.to_string(),
                ..Default::default()
            },
            ProviderResponse {
                tool_calls: vec![ParsedToolCall {
                    tool_name: "list_agents".to_string(),
                    arguments: serde_json::json!({}),
                    call_id: None,
                }],
                ..Default::default()
            },
        ])),
        dispatches: Mutex::new(0),
    });
    let router = scripted_router(&script).await;
    let store: Arc<dyn TaskStore> = Arc::new(
        SqliteTaskStore::new_in_memory()
            .await
            .expect("in-memory task store"),
    );
    let executor = Arc::new(TaskExecutor::new(
        store.clone(),
        TaskExecutorConfig::default(),
    ));
    let metadata = AgentMetadata {
        name: "analyst".to_string(),
        purpose: "You are the analyst. Answer what you are asked.".to_string(),
        ..Default::default()
    };
    // The result is read from the completion notice the handler gossips to
    // the commander. It carries exactly what the task was completed with, and
    // unlike the task row it is not rewritten by the conductor's detached
    // progress updates.
    let commander = AgentKeypair::generate();
    let bus = Arc::new(LearningBus::new(
        "analyst".to_string(),
        "test-swarm".to_string(),
        Arc::new(AgentKeypair::generate()),
        GossipConfig::default(),
    ));
    bus.add_peer("commander".to_string(), commander.public_key().clone())
        .await;
    let mut gossiped = bus.subscribe_gossip_out();

    handle_message_send(
        &Arc::new(MetricsCollector::new(false)),
        &RateLimiter::new(RateLimitConfig::default()),
        &executor,
        &store,
        &Arc::new(McpRegistry::new()),
        &Arc::new(Conductor::new(InMemoryTaskStore::new())),
        Some(&router),
        Some(&bus),
        None,
        Some(ModelChoice::Grok47),
        &arkavo_budget::new_shared_compute_budget(),
        Some(&Arc::new(arkavo_mcp_mesh::MeshToolsState::new())),
        &Arc::new(tokio::sync::RwLock::new(metadata)),
        &Arc::new(tokio::sync::RwLock::new(ToolMemory::new(10))),
        Arc::new(tokio::sync::Mutex::new(None)),
        #[cfg(feature = "iroh")]
        None,
        MessageSendRequest {
            message: Message {
                parts: vec![MessagePart::Text {
                    content: "Which channel should we cut?".to_string(),
                }],
                metadata: None,
            },
            task_id: None,
        },
    )
    .await
    .expect("message accepted");

    let notice = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let (_peer, message) = gossiped.recv().await.expect("gossip channel open");
            if let GossipMessage::TaskCompleted(notice) = message {
                return notice;
            }
        }
    })
    .await
    .expect("the task finished");

    assert!(notice.succeeded, "{}", notice.content);
    assert_eq!(notice.content, answer);
    assert_eq!(*script.dispatches.lock().expect("dispatch count"), 1);
}
