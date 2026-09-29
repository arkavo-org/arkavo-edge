//! A running agent loop for tests that must not load a model.
//!
//! The model is the script in [`model`]; this module runs the real loop
//! against it.

// `pub(crate)` here trips clippy's `redundant_pub_crate` and plain `pub` trips
// rustc's `unreachable_pub`; the test crates in this repo settle it the same
// way, as a module shared by test binaries is never reachable outside.
#![allow(unreachable_pub)]

mod model;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use arkavo_llm::{ParsedToolCall, ProviderResponse};
use arkavo_protocol::agent_config::AgentMode;
use arkavo_server::server::{
    AgentEvent, AgentLoopConfig, CorrelationId, CycleOutcome, run_agent_loop,
};
use tokio::sync::{mpsc, oneshot};

use model::{SCRIPTED_MODEL, scripted_router};
pub use model::{Script, text};

/// A reply that says nothing and calls `tool_name`.
pub fn tool_call(tool_name: &str) -> ProviderResponse {
    ProviderResponse {
        tool_calls: vec![ParsedToolCall {
            tool_name: tool_name.to_string(),
            arguments: serde_json::json!({}),
            call_id: None,
        }],
        ..Default::default()
    }
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

    /// Send `content` as an A2A request and wait for the cycle's answer.
    pub async fn ask(&self, content: &str) -> CycleOutcome {
        let (reply, _receipt) = oneshot::channel();
        let (outcome, answer) = oneshot::channel();
        let sent = self
            .events
            .send(AgentEvent::IncomingMessage {
                sender: "did:key:requester".to_string(),
                content: content.to_string(),
                task_id: uuid::Uuid::new_v4(),
                correlation_id: CorrelationId(uuid::Uuid::new_v4()),
                reply,
                outcome,
            })
            .await;
        assert!(sent.is_ok(), "agent loop accepts events");
        tokio::time::timeout(Duration::from_secs(30), answer)
            .await
            .expect("cycle answered in time")
            .expect("cycle answered")
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
