//! A scripted model and a running agent loop for tests that must not load one.
//!
//! The router resolves every dispatch through an installed
//! `ProviderFactory`, so a script of replies stands in for the model and the
//! test can assert on what the model was asked and how often.

// `pub(crate)` here trips clippy's `redundant_pub_crate` and plain `pub` trips
// rustc's `unreachable_pub`; the test crates in this repo settle it the same
// way, as a module shared by test binaries is never reachable outside.
#![allow(unreachable_pub)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arkavo_llm::{Message, Provider, ProviderResponse, StreamResponse};
use arkavo_protocol::agent_config::AgentMode;
use arkavo_router::{ModelChoice, ProviderFactory};
use arkavo_server::server::{AgentEvent, AgentLoopConfig, run_agent_loop};
use tokio::sync::mpsc;

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
const SCRIPTED_MODEL: ModelChoice = ModelChoice::Grok47;

/// A router whose only arm is the script.
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

/// An agent loop running against a script.
pub struct RunningAgent {
    script: Arc<Script>,
    events: mpsc::Sender<AgentEvent>,
    tick: Arc<AtomicU64>,
    handle: tokio::task::JoinHandle<()>,
}

impl RunningAgent {
    pub async fn start(mode: AgentMode, has_mcp_tools: bool, script: Arc<Script>) -> Self {
        let tick = Arc::new(AtomicU64::new(0));
        let (events, event_rx) = mpsc::channel(32);
        let metadata = arkavo_server::AgentMetadata {
            name: "analyst".to_string(),
            mode: mode.clone(),
            ..Default::default()
        };
        let config = AgentLoopConfig {
            conductor: Arc::new(arkavo_hrm::Conductor::new(
                arkavo_hrm::store::InMemoryTaskStore::new(),
            )),
            router: scripted_router(&script).await,
            mcp_registry: Arc::new(arkavo_protocol::mcp_registry::McpRegistry::new()),
            agent_memory: Arc::new(tokio::sync::RwLock::new(arkavo_server::ToolMemory::new(10))),
            learning_bus: None,
            mesh_state: Arc::new(arkavo_mcp_mesh::MeshToolsState::new()),
            compute_budget: arkavo_budget::new_shared_compute_budget(),
            model_hint: Some(SCRIPTED_MODEL),
            purpose: "You are the analyst. Answer what you are asked.".to_string(),
            orchestrator_tick: tick.clone(),
            has_mcp_tools,
            tool_loop_budget: None,
            total_ram_bytes: 16 * 1024 * 1024 * 1024,
            self_agent_id: "analyst".to_string(),
            commander_model: String::new(),
            agent_mode: mode,
            inference_active: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            context_snapshot: Arc::new(tokio::sync::RwLock::new(None)),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(metadata)),
            #[cfg(feature = "iroh")]
            iroh_node: None,
        };
        let handle = tokio::spawn(run_agent_loop(config, event_rx));
        Self {
            script,
            events,
            tick,
            handle,
        }
    }

    /// Ticks the loop has started so far.
    pub fn ticks(&self) -> u64 {
        self.tick.load(Ordering::Relaxed)
    }

    /// Wait until the loop has started its `count`th tick.
    ///
    /// A tick runs to completion before the loop looks at its next event, so
    /// anything sent after this returns is handled after that tick's work.
    pub async fn ticked(&self, count: u64) {
        tokio::time::timeout(Duration::from_secs(30), async {
            while self.ticks() < count {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("agent loop kept ticking");
    }

    /// Stop the loop and wait for it, so every count the test reads is final.
    pub async fn stop(self) -> Arc<Script> {
        let sent = self.events.send(AgentEvent::Shutdown).await;
        assert!(sent.is_ok(), "agent loop accepts events");
        tokio::time::timeout(Duration::from_secs(30), self.handle)
            .await
            .expect("agent loop stopped in time")
            .expect("agent loop did not panic");
        self.script
    }
}
