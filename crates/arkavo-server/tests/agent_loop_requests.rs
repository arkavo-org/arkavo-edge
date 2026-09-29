//! What the agent loop spends inference on, and what a requester gets back.
//!
//! Each test runs the real loop and conductor against a scripted model, so a
//! dispatch count is a fact about the loop and not about a model's mood.

// The Tokio test entrypoint owns its runtime.
#![allow(clippy::disallowed_methods)]

mod support;

use arkavo_protocol::agent_config::AgentMode;
use arkavo_server::server::CycleOutcome;
use support::{RunningAgent, Script, text, tool_call};

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

const TOOL_NUDGE: &str = "You MUST use a tool now";

/// Regression: the model answered a delegated task in 1,527 characters of
/// text. The planner told it to call a tool instead, it called `list_agents`,
/// and the requester was handed the `list_agents` JSON.
#[tokio::test]
async fn a_text_answer_is_what_the_requester_receives() {
    let answer = "Organic search drives 62% of signups; paid social converts worst.";
    let script = Script::new(vec![text(answer), tool_call("list_agents")]);
    let agent = RunningAgent::start(AgentMode::Specialist, false, script).await;

    let outcome = agent.ask("Which channel should we cut?").await;
    let script = agent.stop().await;

    assert_eq!(
        outcome,
        CycleOutcome::Completed {
            text: answer.to_string()
        }
    );
    assert_eq!(script.dispatches(), 1, "the answer needed no second round");
}

/// The nudge stays where it is wanted: an orchestrator cycle nobody asked for
/// is supposed to act, and a round that only talks is pushed toward a tool.
#[tokio::test]
async fn an_autonomous_cycle_is_still_nudged_toward_a_tool() {
    let script = Script::new(vec![text("I should look around first."), text("Done.")]);
    let agent = RunningAgent::start(AgentMode::Orchestrator, true, script).await;

    agent.ticked(1).await;
    let script = agent.stop().await;

    assert_eq!(script.dispatches(), 2, "round 0, then the nudged round");
    assert!(!script.prompt(0).contains(TOOL_NUDGE));
    assert!(
        script.prompt(1).contains(TOOL_NUDGE),
        "got {}",
        script.prompt(1)
    );
}

/// Regression: a specialist's text answers counted as "no action", so after
/// three of them the dead-man switch ran cycles of its own and told the model
/// to act NOW with nothing delegated.
#[tokio::test]
async fn a_specialist_that_answers_in_text_is_not_pushed_to_act() {
    let script = Script::new(vec![
        text("first answer"),
        text("second answer"),
        text("third answer"),
    ]);
    let agent = RunningAgent::start(AgentMode::Specialist, false, script).await;

    for expected in ["first answer", "second answer", "third answer"] {
        assert_eq!(
            agent.ask("What should we measure?").await,
            CycleOutcome::Completed {
                text: expected.to_string()
            }
        );
    }
    // One more tick with nothing delegated: this is where the switch fired.
    agent.ticked(agent.ticks() + 1).await;
    let script = agent.stop().await;

    assert_eq!(script.dispatches(), 3, "one dispatch per delegated task");
    for index in 0..script.dispatches() {
        let prompt = script.prompt(index);
        assert!(
            !prompt.contains("You MUST take an action NOW"),
            "dispatch {index} was pushed to act: {prompt}"
        );
    }
}
