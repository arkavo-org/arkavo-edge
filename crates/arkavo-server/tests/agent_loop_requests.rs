//! What the agent loop spends inference on, and what a requester gets back.
//!
//! Each test runs the real loop and conductor against a scripted model, so a
//! dispatch count is a fact about the loop and not about a model's mood.

// The Tokio test entrypoint owns its runtime.
#![allow(clippy::disallowed_methods)]

mod support;

use arkavo_protocol::agent_config::AgentMode;
use support::{RunningAgent, Script, text};

/// Regression: in specialist mode the agent ran a full planner cycle on
/// `Continue.` at startup and made a tool call before any task had arrived.
#[tokio::test]
async fn a_specialist_spends_no_inference_before_a_task_arrives() {
    let script = Script::new(vec![text("unprompted")]);
    let agent = RunningAgent::start(AgentMode::Specialist, false, script).await;

    agent.ticked(1).await;
    let script = agent.stop().await;

    assert_eq!(
        script.dispatches(),
        0,
        "the first tick had nothing delegated to it"
    );
}
