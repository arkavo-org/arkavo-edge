//! An agent as `message/send` and `tasks/get` see it, for tests.
//!
//! The model is a script, so what the agent was asked is a fact a test can
//! read back. `serve` puts the agent behind a real JSON-RPC server on
//! loopback, where it is talked to the way any client talks to an agent.

#[path = "../../../../tests/support/model.rs"]
mod model;

use std::sync::Arc;
use std::time::Duration;

use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_llm::{Message as Prompt, Provider, ProviderResponse, StreamResponse};
use arkavo_mcp_mesh::MeshToolsState;
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::MetricsCollector;
use arkavo_protocol::rate_limit::{RateLimitConfig, RateLimiter};
use arkavo_protocol::types::{
    Message, MessagePart, MessageSendRequest, MessageSendResponse, TaskGetRequest, TaskStatus,
};
use arkavo_router::{ModelChoice, ProviderFactory};
use arkavo_tasks::task_executor::{TaskExecutor, TaskExecutorConfig};
use arkavo_tasks::task_store::{SqliteTaskStore, TaskStore};
use arkavo_tasks::types::Task;
use jsonrpsee::RpcModule;
use jsonrpsee::server::{Server, ServerHandle};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock, mpsc};

use super::super::tasks::handle_tasks_get;
use super::{PlanSource, handle_message_send_from};
use crate::server::agent_event::AgentEvent;
use crate::server::config_helpers::AgentMetadata;
use crate::server::pipeline::Driving;
use crate::server::tool_memory::ToolMemory;
use model::{SCRIPTED_MODEL, scripted_router};
pub(super) use model::{Script, text};

pub(super) fn purpose(role: &str) -> String {
    format!("You are the {role}. Do the {role}'s work.")
}

/// One agent process, as far as `message/send` and `tasks/get` see it.
pub(super) struct Agent {
    plans: PlanSource,
    router: Option<Arc<arkavo_router::Router>>,
    metrics: Arc<MetricsCollector>,
    rate_limiter: RateLimiter,
    executor: Arc<TaskExecutor>,
    pub(super) store: Arc<dyn TaskStore>,
    pub(super) mesh: Arc<MeshToolsState>,
    metadata: Arc<RwLock<AgentMetadata>>,
    memory: Arc<RwLock<ToolMemory>>,
    events: Arc<Mutex<Option<mpsc::Sender<AgentEvent>>>>,
}

impl Agent {
    /// An agent that answers for itself, as one started without a pipeline
    /// kit does.
    pub(super) async fn new(
        role: &'static str,
        router: Option<Arc<arkavo_router::Router>>,
    ) -> Self {
        let store: Arc<dyn TaskStore> = Arc::new(
            SqliteTaskStore::new_in_memory()
                .await
                .expect("in-memory task store"),
        );
        Self {
            plans: PlanSource::Decided(Driving::No),
            router,
            metrics: Arc::new(MetricsCollector::new(false)),
            rate_limiter: RateLimiter::new(RateLimitConfig::default()),
            executor: Arc::new(TaskExecutor::new(
                store.clone(),
                TaskExecutorConfig::default(),
            )),
            store,
            mesh: Arc::new(MeshToolsState::new()),
            metadata: Arc::new(RwLock::new(AgentMetadata {
                name: role.to_string(),
                role_id: Some(role.to_string()),
                purpose: purpose(role),
                ..Default::default()
            })),
            memory: Arc::new(RwLock::new(ToolMemory::new(10))),
            events: Arc::new(Mutex::new(None)),
        }
    }

    pub(super) async fn scripted(role: &'static str, script: &Arc<Script>) -> Self {
        Self::new(role, Some(scripted_router(script).await)).await
    }

    /// The same agent with `driving` as what its kit makes of it.
    pub(super) fn driving(mut self, driving: Driving) -> Self {
        self.plans = PlanSource::Decided(driving);
        self
    }

    /// Give the agent an agent loop to route messages to, and return the
    /// end a loop would read from.
    pub(super) async fn with_agent_loop(&self) -> mpsc::Receiver<AgentEvent> {
        let (tx, rx) = mpsc::channel(8);
        *self.events.lock().await = Some(tx);
        rx
    }

    pub(super) async fn message_send(
        &self,
        request: MessageSendRequest,
    ) -> Result<MessageSendResponse, jsonrpsee::types::ErrorObjectOwned> {
        handle_message_send_from(
            &self.metrics,
            &self.rate_limiter,
            &self.executor,
            &self.store,
            &Arc::new(McpRegistry::new()),
            &Arc::new(Conductor::new(InMemoryTaskStore::new())),
            self.router.as_ref(),
            None,
            None,
            Some(SCRIPTED_MODEL),
            &arkavo_budget::new_shared_compute_budget(),
            Some(&self.mesh),
            &self.metadata,
            &self.memory,
            self.events.clone(),
            #[cfg(feature = "iroh")]
            None,
            &self.plans,
            request,
        )
        .await
    }

    pub(super) async fn send(&self, content: &str, metadata: Option<Value>) -> uuid::Uuid {
        let sent = self
            .message_send(MessageSendRequest {
                message: message(content, metadata),
                task_id: None,
            })
            .await
            .expect("message accepted");
        assert_eq!(sent.status, TaskStatus::Submitted);
        uuid::Uuid::parse_str(&sent.task_id).expect("task id")
    }

    /// The task once it has reached a state a requester stops polling at.
    pub(super) async fn finished(&self, task_id: uuid::Uuid) -> Task {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let task = self
                    .store
                    .get_task(&task_id)
                    .await
                    .expect("task store readable")
                    .expect("task exists");
                if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
                    return task;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the task finished")
    }

    /// Put the agent behind a JSON-RPC server on loopback.
    pub(super) async fn serve(self: &Arc<Self>) -> (String, ServerHandle) {
        let mut module = RpcModule::new(self.clone());
        module
            .register_async_method("message/send", |params, agent, _| async move {
                agent.message_send(params.one()?).await
            })
            .expect("message/send registers");
        module
            .register_async_method("tasks/get", |params, agent, _| async move {
                let request: TaskGetRequest = params.one()?;
                handle_tasks_get(&agent.metrics, &agent.rate_limiter, &agent.store, request).await
            })
            .expect("tasks/get registers");
        let server = Server::builder()
            .build("127.0.0.1:0")
            .await
            .expect("loopback server");
        let address = format!("http://{}", server.local_addr().expect("bound address"));
        (address, server.start(module))
    }
}

pub(super) fn message(content: &str, metadata: Option<Value>) -> Message {
    Message {
        parts: vec![MessagePart::Text {
            content: content.to_string(),
        }],
        metadata,
    }
}

pub(super) fn step_marker(run_id: &str) -> Value {
    json!({
        "source": "pipeline",
        "pipeline": {"run_id": run_id, "step": 2, "steps": 2, "role": "reviewer"}
    })
}

pub(super) fn result_text(task: &Task) -> String {
    let result: Message =
        serde_json::from_value(task.result.clone().expect("result")).expect("a message");
    match result.parts.as_slice() {
        [MessagePart::Text { content }] => content.clone(),
        parts => panic!("an agent answering alone returns one text part, got {parts:?}"),
    }
}

/// A model that takes `delay` to answer.
pub(super) struct SlowProvider(Duration);

#[async_trait::async_trait]
impl Provider for SlowProvider {
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
        "slow"
    }

    async fn complete_with_tools(
        &self,
        _messages: Vec<Prompt>,
        _tools: Option<Value>,
        _max_tokens: Option<usize>,
    ) -> arkavo_llm::Result<ProviderResponse> {
        tokio::time::sleep(self.0).await;
        Ok(text("An answer nobody is waiting for."))
    }
}

pub(super) struct SlowFactory(Duration);

impl ProviderFactory for SlowFactory {
    fn build(&self, _model: &ModelChoice) -> arkavo_router::Result<Box<dyn Provider>> {
        Ok(Box::new(SlowProvider(self.0)))
    }
}

pub(super) async fn slow_router(delay: Duration) -> Arc<arkavo_router::Router> {
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
            .with_provider_factory(Arc::new(SlowFactory(delay))),
    )
}
