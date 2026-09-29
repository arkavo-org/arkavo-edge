//! A step of someone else's pipeline run, through the handler itself.

use std::time::Duration;

use arkavo_protocol::types::TaskStatus;
use serde_json::json;

use super::test_agent::{Agent, Script, result_text, slow_router, step_marker, text};
use crate::server::agent_event::AgentEvent;

/// Regression: every message went through the agent loop, whose
/// conversation lasts as long as the agent. A step of one run was answered
/// with the steps of earlier runs in front of the model.
#[tokio::test]
async fn a_step_is_answered_without_the_history_of_an_earlier_run() {
    let script = Script::new(vec![
        text("The Q3 plan overspends on paid social."),
        text("The Q4 plan is sound."),
    ]);
    let reviewer = Agent::scripted("reviewer", &script).await;
    let mut agent_loop = reviewer.with_agent_loop().await;

    let first = reviewer
        .send(
            "Review the Q3 plan: spend 80% on paid social.",
            Some(step_marker("run-1")),
        )
        .await;
    let first = reviewer.finished(first).await;
    let second = reviewer
        .send(
            "Review the Q4 plan: spend 20% on search.",
            Some(step_marker("run-2")),
        )
        .await;
    let second = reviewer.finished(second).await;

    assert_eq!(
        result_text(&first),
        "The Q3 plan overspends on paid social."
    );
    assert_eq!(result_text(&second), "The Q4 plan is sound.");
    assert_eq!(script.dispatches(), 2);
    let second_prompt = script.prompt(1);
    assert!(
        second_prompt.contains("Review the Q4 plan"),
        "{second_prompt}"
    );
    assert!(!second_prompt.contains("Q3"), "{second_prompt}");
    assert!(!second_prompt.contains("paid social"), "{second_prompt}");
    assert!(
        agent_loop.try_recv().is_err(),
        "a step never enters the agent loop's conversation"
    );
}

/// Regression: messages queued in the same tick were folded into one cycle
/// and every requester was handed the same answer.
#[tokio::test]
async fn steps_that_arrive_together_are_answered_separately() {
    let script = Script::new(vec![text("first answer"), text("second answer")]);
    let reviewer = Agent::scripted("reviewer", &script).await;
    let mut agent_loop = reviewer.with_agent_loop().await;

    let (a, b) = tokio::join!(
        reviewer.send("Review the alpha draft.", Some(step_marker("run-a"))),
        reviewer.send("Review the beta draft.", Some(step_marker("run-b"))),
    );
    let (a, b) = tokio::join!(reviewer.finished(a), reviewer.finished(b));

    let mut answers = vec![result_text(&a), result_text(&b)];
    answers.sort();
    assert_eq!(answers, ["first answer", "second answer"]);
    assert_eq!(script.dispatches(), 2, "one dispatch per step");
    for index in 0..2 {
        let prompt = script.prompt(index);
        assert_ne!(
            prompt.contains("alpha draft"),
            prompt.contains("beta draft"),
            "each dispatch saw exactly one of the steps: {prompt}"
        );
    }
    assert!(agent_loop.try_recv().is_err());
}

/// An unmarked message to an agent with an agent loop still goes to the
/// loop: only steps and pipeline runs leave it.
#[tokio::test]
async fn an_ordinary_message_still_goes_to_the_agent_loop() {
    let script = Script::new(vec![]);
    let reviewer = Agent::scripted("reviewer", &script).await;
    let mut agent_loop = reviewer.with_agent_loop().await;

    reviewer.send("Is this any good?", None).await;

    let event = tokio::time::timeout(Duration::from_secs(5), agent_loop.recv())
        .await
        .expect("the loop was handed the message")
        .expect("channel open");
    assert!(
        matches!(&event, AgentEvent::IncomingMessage { content, .. } if content == "Is this any good?")
    );
    assert_eq!(
        script.dispatches(),
        0,
        "the handler did not answer it itself"
    );
}

/// The sender says how long it will wait. Work that goes on past that is
/// work for nobody, on a machine the next run needs.
#[tokio::test]
async fn a_step_stops_when_its_sender_has_stopped_waiting() {
    let reviewer = Agent::new("reviewer", Some(slow_router(Duration::from_secs(30)).await)).await;
    let mut marker = step_marker("run-1");
    marker["pipeline"]["timeout_ms"] = json!(1200);

    let started = std::time::Instant::now();
    let task_id = reviewer.send("Review this draft.", Some(marker)).await;
    let task = reviewer.finished(task_id).await;

    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(
        task.error.expect("error").message,
        "the pipeline step did not finish within the 1s its sender allowed"
    );
}
