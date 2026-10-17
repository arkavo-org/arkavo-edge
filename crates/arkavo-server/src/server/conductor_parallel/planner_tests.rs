//! The planner and executor tracks, driven by a scripted model.

use std::time::Duration;

use arkavo_llm::ParsedToolCall;
use arkavo_router::ModelChoice;

use super::*;
use crate::server::scripted_model::{Script, calling, calling_all, scripted_router, text};

/// Run the planner on `script` with an executor whose calls each finish
/// `tool_time` after the one before, reported one at a time as the judge
/// reports them.
async fn plan_with_slow_tools(script: &Arc<Script>, tool_time: Duration) -> PlanResult {
    let router = scripted_router(script).await;
    let (plan_tx, mut plan_rx) = mpsc::channel::<PlannedActions>(4);
    let (feedback_tx, feedback_rx) = mpsc::channel(4);
    let executor = tokio::spawn(async move {
        while let Some(planned) = plan_rx.recv().await {
            for call in planned.tool_calls {
                tokio::time::sleep(tool_time).await;
                let feedback = JudgeFeedback {
                    tool_results: vec![CondensedToolResult {
                        content: format!("{} finished", call.tool_name),
                        call_id: call.call_id.unwrap_or_default(),
                        tool_name: call.tool_name,
                    }],
                    should_replan: false,
                };
                if feedback_tx.send(feedback).await.is_err() {
                    return;
                }
            }
        }
    });
    let result = planner_track(
        &router,
        &Arc::new(ToolRegistry::empty()),
        &Arc::new(McpRegistry::new()),
        "Make a gold coin sprite",
        vec![arkavo_llm::Message::user("Make a gold coin sprite")],
        Some(&ModelChoice::Grok47),
        plan_tx,
        feedback_rx,
        None,
    )
    .await;
    executor.await.expect("executor");
    result
}

/// Regression (SRV-014): generate_image answers after up to 90 s, and the
/// planner gave up on its result after the same 90 s, its inference
/// timeout. The result never reached the model, so a requester got the
/// tool record instead of a reply.
#[arkavo_test_macros::spec("SRV-014")]
#[tokio::test(start_paused = true)]
async fn a_tool_slower_than_an_inference_round_still_reaches_the_model() {
    let answer = "Your gold coin sprite is on its way.";
    let script = Script::of([calling("generate_image"), text(answer)]);

    let result = for_requester(plan_with_slow_tools(&script, Duration::from_secs(120))).await;

    assert_eq!(result.failure, None);
    assert_eq!(result.final_text, answer);
    assert_eq!(*script.dispatches.lock().expect("dispatch count"), 2);
}

/// Regression (SRV-014): the judge reports each call of a batch on its
/// own, and the planner went on after the first report. A denied call is
/// reported at once while generate_image takes minutes, so its result
/// reached nobody and the model answered without it.
#[arkavo_test_macros::spec("SRV-014")]
#[tokio::test(start_paused = true)]
async fn the_next_round_waits_for_every_call_in_the_batch() {
    let script = Script::of([
        calling_all(&["shell_exec", "generate_image"]),
        text("Your gold coin sprite is on its way."),
    ]);

    for_requester(plan_with_slow_tools(&script, Duration::from_secs(120))).await;

    let prompts = script.prompts.lock().expect("prompts");
    assert!(prompts[1].contains("shell_exec finished"), "{}", prompts[1]);
    assert!(
        prompts[1].contains("generate_image finished"),
        "{}",
        prompts[1]
    );
}

/// Regression (SRV-014): the planner now waits for one result per call, so
/// a call the model repeated in its batch is run and counted once, or the
/// planner would wait for a result that never comes.
#[arkavo_test_macros::spec("SRV-014")]
#[tokio::test(start_paused = true)]
async fn a_repeated_call_is_run_once() {
    let script = Script::of([
        calling_all(&["list_agents", "list_agents"]),
        text("Two agents are on the mesh."),
    ]);

    let result = for_requester(plan_with_slow_tools(&script, Duration::ZERO)).await;

    assert_eq!(result.failure, None);
    let prompts = script.prompts.lock().expect("prompts");
    assert_eq!(prompts[1].matches("list_agents finished").count(), 1);
}

/// SRV-014: an autonomous cycle is not owed an answer, so its rounds end on
/// their tool calls with no extra round.
#[arkavo_test_macros::spec("SRV-014")]
#[tokio::test(start_paused = true)]
async fn an_autonomous_cycle_gets_no_answer_round() {
    let script = Script::of([calling("list_agents"), calling("list_agents")]);

    let result = plan_with_slow_tools(&script, Duration::ZERO).await;

    assert_eq!(result.final_text, "");
    assert_eq!(*script.dispatches.lock().expect("dispatch count"), 2);
}

/// SRV-014: a requester whose rounds ended on tool calls gets one more
/// round, offered no tools, whose text is the answer.
#[arkavo_test_macros::spec("SRV-014")]
#[tokio::test(start_paused = true)]
async fn a_requester_gets_an_answer_round_without_tools() {
    let answer = "Two agents are on the mesh.";
    let script = Script::of([calling("list_agents"), calling("list_agents"), text(answer)]);

    let result = for_requester(plan_with_slow_tools(&script, Duration::ZERO)).await;

    assert_eq!(result.final_text, answer);
    assert_eq!(
        *script.offered_tools.lock().expect("tool offers"),
        [true, true, false]
    );
}

/// SRV-014: an answer round that calls a tool anyway answers nothing, and
/// the requester falls back to the record of the tool results.
#[arkavo_test_macros::spec("SRV-014")]
#[tokio::test(start_paused = true)]
async fn an_answer_round_that_calls_a_tool_answers_nothing() {
    let script = Script::of([
        calling("list_agents"),
        calling("list_agents"),
        calling("list_agents"),
    ]);

    let result = for_requester(plan_with_slow_tools(&script, Duration::ZERO)).await;

    assert_eq!(result.final_text, "");
    assert_eq!(*script.dispatches.lock().expect("dispatch count"), 3);
}

/// Regression: a denied call was dropped without a result, so a batch of
/// denied calls left the planner waiting for feedback that never came. It
/// is reported as the sequential loop reports it.
#[arkavo_test_macros::spec("SRV-009")]
#[tokio::test]
async fn a_denied_call_is_reported_to_the_model() {
    let script = Script::of([]);
    let router = scripted_router(&script).await;
    let (plan_tx, plan_rx) = mpsc::channel(1);
    let (result_tx, mut result_rx) = mpsc::channel(1);
    let (obs_tx, _observations) = mpsc::channel(1);
    plan_tx
        .send(PlannedActions {
            tool_calls: vec![ParsedToolCall {
                tool_name: "shell_exec".to_string(),
                arguments: serde_json::json!({"command": "ls"}),
                call_id: Some("call_7".to_string()),
            }],
        })
        .await
        .expect("the executor is listening");
    drop(plan_tx);

    executor_track(
        &router,
        &Arc::new(ToolRegistry::empty()),
        &Arc::new(McpRegistry::new()),
        plan_rx,
        result_tx,
        None,
        None,
        &[],
        obs_tx,
        Some(&std::collections::HashSet::from(
            ["list_agents".to_string()],
        )),
        #[cfg(feature = "taint")]
        None,
    )
    .await;

    let reported = result_rx.recv().await.expect("the denial is reported");
    assert_eq!(reported.tool_name, "shell_exec");
    assert_eq!(reported.call_id, "call_7");
    assert!(!reported.success);
    assert!(reported.result.contains("(Denied)"), "{}", reported.result);
}
