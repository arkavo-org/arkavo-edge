//! What an agent does with a request as its compute budget windows end.
//!
//! The budget reads a clock the test moves, so a ten-minute window ends when
//! the test says so.

// The Tokio test entrypoint owns its runtime.
#![allow(clippy::disallowed_methods)]

#[path = "support/model.rs"]
mod model;

use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arkavo_budget::{BudgetAllocation, BudgetClock, SharedComputeBudget};
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_mcp_mesh::MeshToolsState;
use arkavo_mcp_tools::ToolRegistry;
use arkavo_protocol::agent_config::AgentMode;
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_server::execute_with_conductor_and_learning;
use arkavo_server::server::{
    AgentEvent, AgentLoopConfig, CorrelationId, CycleOutcome, run_agent_loop,
};
use model::{SCRIPTED_MODEL, Script, scripted_router, text};
use tokio::sync::{mpsc, oneshot};

const QUESTION: &str = "Which channel should we cut?";
const ANSWER: &str = "Organic search drives 62% of signups; paid social converts worst.";

/// A clock that stands still until it is told how much time has passed.
struct Elapsed(Arc<Mutex<Duration>>);

impl Elapsed {
    fn none() -> Self {
        Self(Arc::new(Mutex::new(Duration::ZERO)))
    }

    fn clock(&self) -> BudgetClock {
        let start = Instant::now();
        let elapsed = self.0.clone();
        BudgetClock::new(move || start + *elapsed.lock().expect("elapsed time"))
    }

    fn pass(&self, seconds: u64) {
        *self.0.lock().expect("elapsed time") += Duration::from_secs(seconds);
    }
}

/// The window an agent grants itself.
fn window() -> BudgetAllocation {
    BudgetAllocation::self_managed()
}

async fn spend_every_inference(budget: &SharedComputeBudget) {
    let mut budget = budget.write().await;
    for _ in 0..window().max_inferences {
        budget.consume_inference(10, 0.0);
    }
}

/// An agent loop with no MCP tools, which is the kind that checks its compute
/// budget before it serves anything.
struct Agent {
    events: mpsc::Sender<AgentEvent>,
    handle: tokio::task::JoinHandle<()>,
}

impl Agent {
    async fn start(script: &Arc<Script>, compute_budget: SharedComputeBudget) -> Self {
        let (events, event_rx) = mpsc::channel(32);
        let metadata = arkavo_server::AgentMetadata {
            name: "analyst".to_string(),
            mode: AgentMode::Specialist,
            ..Default::default()
        };
        let config = AgentLoopConfig {
            conductor: Arc::new(Conductor::new(InMemoryTaskStore::new())),
            router: scripted_router(script).await,
            mcp_registry: Arc::new(McpRegistry::new()),
            agent_memory: Arc::new(tokio::sync::RwLock::new(arkavo_server::ToolMemory::new(10))),
            learning_bus: None,
            mesh_state: Arc::new(MeshToolsState::new()),
            compute_budget,
            model_hint: Some(SCRIPTED_MODEL),
            purpose: "You are the analyst. Answer what you are asked.".to_string(),
            orchestrator_tick: Arc::new(AtomicU64::new(0)),
            has_mcp_tools: false,
            tool_loop_budget: None,
            total_ram_bytes: 16 * 1024 * 1024 * 1024,
            self_agent_id: "analyst".to_string(),
            commander_model: String::new(),
            agent_mode: AgentMode::Specialist,
            inference_active: Arc::new(AtomicBool::new(false)),
            context_snapshot: Arc::new(tokio::sync::RwLock::new(None)),
            agent_metadata: Arc::new(tokio::sync::RwLock::new(metadata)),
            #[cfg(feature = "iroh")]
            iroh_node: None,
        };
        Self {
            events,
            handle: tokio::spawn(run_agent_loop(config, event_rx)),
        }
    }

    /// Send a request that carries no budget allocation and wait for what the
    /// cycle that served it produced.
    async fn ask(&self, content: &str) -> CycleOutcome {
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

    async fn stop(self) {
        let sent = self.events.send(AgentEvent::Shutdown).await;
        assert!(sent.is_ok(), "agent loop accepts events");
        tokio::time::timeout(Duration::from_secs(30), self.handle)
            .await
            .expect("agent loop stopped in time")
            .expect("agent loop did not panic");
    }
}

/// Which loop the conductor runs a request through.
enum Tools {
    /// No tools at all: the single-track tool loop.
    None,
    /// The mesh tools a specialist is given: the parallel planner.
    Mesh,
}

/// Run one request through the conductor, as `message/send` does when no
/// agent loop is running.
async fn conduct(
    script: &Arc<Script>,
    budget: &SharedComputeBudget,
    tools: Tools,
) -> Result<String, String> {
    let router = scripted_router(script).await;
    // The tool loop routes as the host's own work, which the operator
    // approves once; the planner names the model and is not asked.
    router.approve_cloud_for_host();
    let mesh_state = Arc::new(MeshToolsState::new());
    let mut registry = ToolRegistry::empty();
    if matches!(tools, Tools::Mesh) {
        arkavo_mcp_mesh::register_tools(&mut registry, mesh_state.clone());
    }
    execute_with_conductor_and_learning(
        &Arc::new(Conductor::new(InMemoryTaskStore::new())),
        &router,
        &Arc::new(McpRegistry::new()),
        QUESTION.to_string(),
        None,
        None,
        None,
        None,
        None,
        Some(&mesh_state),
        Some(&SCRIPTED_MODEL),
        None,
        Some(budget),
        None,
        true,
        Some(Arc::new(registry)),
        None,
        #[cfg(feature = "iroh")]
        None,
    )
    .await
}

/// Regression: nothing gave an agent a second window. Ten minutes after it
/// started, an agent with no MCP tools answered every request that carried
/// no allocation with "compute budget exhausted".
#[tokio::test]
async fn an_agent_idle_past_its_window_still_answers() {
    let time = Elapsed::none();
    let budget = arkavo_budget::new_shared_compute_budget_on(time.clock());
    let script = Script::new(vec![text(ANSWER)]);
    let agent = Agent::start(&script, budget).await;

    time.pass(window().ttl_secs + 1);
    let outcome = agent.ask(QUESTION).await;
    agent.stop().await;

    assert_eq!(
        outcome,
        CycleOutcome::Completed {
            text: ANSWER.to_string()
        }
    );
    assert_eq!(script.dispatches(), 1);
    assert!(script.prompt(0).contains(QUESTION));
}

/// The same fault where no agent loop runs and the handler conducts the task
/// itself: the planner refused before its first round.
#[tokio::test]
async fn a_request_conducted_after_the_window_ended_is_served() {
    let time = Elapsed::none();
    let budget = arkavo_budget::new_shared_compute_budget_on(time.clock());
    spend_every_inference(&budget).await;
    let script = Script::new(vec![text(ANSWER)]);

    time.pass(window().ttl_secs);
    let outcome = conduct(&script, &budget, Tools::Mesh).await;

    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(script.dispatches() >= 1, "the model was asked");
    // Read as the code that looks at the counters reads them: the window has
    // to have been started, not only reported.
    assert_eq!(
        budget.read().await.remaining_inferences,
        window().max_inferences
    );
}

/// Regression: the tool loop refilled a budget that had no inferences left,
/// on the spot, so the budget limited nothing on that path.
#[tokio::test]
async fn a_spent_budget_stops_work_until_its_window_ends() {
    let time = Elapsed::none();
    let budget = arkavo_budget::new_shared_compute_budget_on(time.clock());
    spend_every_inference(&budget).await;
    let script = Script::new(vec![text(ANSWER)]);

    time.pass(window().ttl_secs - 1);
    let refused = conduct(&script, &budget, Tools::None).await;

    let reason = refused.expect_err("nothing left to spend");
    assert!(reason.contains("compute budget exhausted"), "got {reason}");
    assert_eq!(script.dispatches(), 0, "the model was not asked");
    assert_eq!(budget.read().await.snapshot().remaining_inferences, 0);

    time.pass(1);
    let served = conduct(&script, &budget, Tools::None).await;

    assert_eq!(served.as_deref(), Ok(ANSWER));
    assert_eq!(script.dispatches(), 1);
    assert_eq!(
        budget.read().await.snapshot().remaining_inferences,
        window().max_inferences - 1
    );
}

/// Regression: a window that ended with inferences unspent was treated by the
/// tool loop as exhausted for good, as it only refilled a count of zero.
#[tokio::test]
async fn an_unspent_window_that_ended_does_not_stop_the_tool_loop() {
    let time = Elapsed::none();
    let budget = arkavo_budget::new_shared_compute_budget_on(time.clock());
    let script = Script::new(vec![text(ANSWER)]);

    time.pass(window().ttl_secs + 1);
    let served = conduct(&script, &budget, Tools::None).await;

    assert_eq!(served.as_deref(), Ok(ANSWER));
    assert_eq!(script.dispatches(), 1);
}

/// A caller's allocation still paces the agent for as long as it was granted.
#[tokio::test]
async fn a_grant_that_is_spent_stops_work_until_it_lapses() {
    let time = Elapsed::none();
    let budget = arkavo_budget::new_shared_compute_budget_on(time.clock());
    let script = Script::new(vec![text("first"), text("second")]);
    budget.write().await.refresh(&BudgetAllocation {
        max_inferences: 1,
        ttl_secs: 120,
        ..BudgetAllocation::default()
    });

    let first = conduct(&script, &budget, Tools::None).await;
    let refused = conduct(&script, &budget, Tools::None).await;

    assert_eq!(first.as_deref(), Ok("first"));
    let reason = refused.expect_err("the grant was for one inference");
    assert!(reason.contains("compute budget exhausted"), "got {reason}");
    assert_eq!(script.dispatches(), 1);

    time.pass(120);
    let second = conduct(&script, &budget, Tools::None).await;

    assert_eq!(second.as_deref(), Ok("second"));
}
