//! What a requester gets back from `message/send` when no agent loop is
//! running and the handler executes the task itself.

use std::sync::Arc;
use std::time::Duration;

use arkavo_crypto::AgentKeypair;
use arkavo_gossip::{GossipConfig, GossipMessage, TaskCompletionNotice};
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::MetricsCollector;
use arkavo_protocol::rate_limit::{RateLimitConfig, RateLimiter};
use arkavo_protocol::types::{Message, MessagePart, MessageSendRequest};
use arkavo_router::ModelChoice;
use arkavo_tasks::task_executor::{TaskExecutor, TaskExecutorConfig};
use arkavo_tasks::task_store::{SqliteTaskStore, TaskStore};

use super::handle_message_send;
use crate::server::LearningBus;
use crate::server::config_helpers::AgentMetadata;
use crate::server::scripted_model::{Script, calling, scripted_router, text};
use crate::server::tool_memory::ToolMemory;

/// Regression: the planner answered a text reply with "You MUST use a tool
/// now", so the requester was handed the output of `list_agents` instead of
/// the answer. This is the same fault on the path that runs without an agent
/// loop.
#[tokio::test]
async fn a_text_answer_is_the_result_of_the_task() {
    let answer = "Organic search drives 62% of signups; paid social converts worst.";
    let script = Script::of([text(answer), calling("list_agents")]);

    let notice = answer_to(&script, "Which channel should we cut?").await;

    assert!(notice.succeeded, "{}", notice.content);
    assert_eq!(notice.content, answer);
    assert_eq!(*script.dispatches.lock().expect("dispatch count"), 1);
}

/// Regression (SRV-014): a writer asked for a sprite called generate_image,
/// then list_agents, and the planner's rounds ran out, so the requester got
/// the truncated JSON of those tool results instead of a reply. A round
/// offered no tools now turns the results into the model's own answer.
#[arkavo_test_macros::spec("SRV-014")]
#[tokio::test]
async fn rounds_that_end_on_tool_calls_are_answered_from_the_results() {
    let answer = "Two agents are on the mesh: the analyst and the artist.";
    let script = Script::of([calling("list_agents"), calling("list_agents"), text(answer)]);

    let notice = answer_to(&script, "Who else is on the mesh?").await;

    assert!(notice.succeeded, "{}", notice.content);
    assert_eq!(notice.content, answer);
    assert_eq!(
        *script.offered_tools.lock().expect("tool offers"),
        [true, true, false],
        "the answer round offers no tools"
    );
}

/// The answer the requester of `question` gets from an analyst whose model
/// follows `script`. It is read from the completion notice the handler
/// gossips to the commander: that carries exactly what the task was
/// completed with, and unlike the task row it is not rewritten by the
/// conductor's detached progress updates.
async fn answer_to(script: &Arc<Script>, question: &str) -> TaskCompletionNotice {
    let router = scripted_router(script).await;
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
    let commander = AgentKeypair::generate();
    let bus = Arc::new(LearningBus::new(
        "analyst".to_string(),
        "test-swarm".to_string(),
        Arc::new(AgentKeypair::generate()),
        GossipConfig::default(),
    ));
    bus.add_peer("commander".to_string(), commander.public_key().clone())
        .await
        .unwrap();
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
        None,
        MessageSendRequest {
            message: Message {
                parts: vec![MessagePart::Text {
                    content: question.to_string(),
                }],
                metadata: None,
            },
            task_id: None,
        },
    )
    .await
    .expect("message accepted");

    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let (_peer, message) = gossiped.recv().await.expect("gossip channel open");
            if let GossipMessage::TaskCompleted(notice) = message {
                return notice;
            }
        }
    })
    .await
    .expect("the task finished")
}
