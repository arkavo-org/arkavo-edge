//! Whether the running cycle owes its text to a requester.
//!
//! The planner is reached through the conductor from two kinds of caller. An
//! autonomous orchestrator cycle is supposed to act, so a round that only
//! talks is pushed toward a tool. A cycle serving a `message/send` is supposed
//! to answer, and prose is an answer: pushing it toward a tool replaces what
//! the requester asked for with whatever the tool returned.
//!
//! The planner cannot tell the two apart from its arguments, so the caller
//! says which one it is by running the cycle inside [`for_requester`].

use std::future::Future;
use std::time::Duration;

use tracing::warn;

/// What the model is told when its rounds ended on tool calls and a
/// requester is still owed an answer. Small models given only "answer from
/// the results" copied the last tool result, JSON and all, as the reply.
const ANSWER_FROM_RESULTS: &str = "Reply to the request now in one or two sentences of your own \
     words: say what you did and what the requester gets. Do not copy the tool output or any \
     JSON, and do not call any tool.";

tokio::task_local! {
    static REQUESTER_WAITING: ();
}

/// Run `cycle` as the answer to a requester waiting on its result.
///
/// The mark follows the cycle through every `await` on the calling task, which
/// is where the conductor runs the planner. Work the conductor spawns onto
/// other tasks does not inherit it and keeps the autonomous behaviour.
pub(in crate::server) async fn for_requester<F: Future>(cycle: F) -> F::Output {
    REQUESTER_WAITING.scope((), cycle).await
}

fn requester_waiting() -> bool {
    REQUESTER_WAITING.try_with(|()| ()).is_ok()
}

/// True when a round that called no tool has already produced the cycle's
/// answer, so the planner must stop instead of asking for a tool call.
pub(super) fn text_is_final(text: &str) -> bool {
    requester_waiting() && !text.trim().is_empty()
}

/// The answer a waiting requester is owed when the planner's rounds ended
/// on tool calls, so the model never said what they found: one more round,
/// offered no tools, in which it does. `None` when nobody is waiting, or
/// when that round fails or answers with a tool call anyway; the caller
/// then falls back to the record of what the tools did.
pub(super) async fn after_tools(
    router: &arkavo_router::Router,
    task_content: &str,
    mut messages: Vec<arkavo_llm::Message>,
    model_hint: Option<&arkavo_router::ModelChoice>,
    timeout: Duration,
) -> Option<String> {
    if !requester_waiting() {
        return None;
    }
    messages.push(arkavo_llm::Message::user(ANSWER_FROM_RESULTS.to_string()));
    let round = router.route_with_tools_execution(task_content, messages, None, model_hint);
    match tokio::time::timeout(timeout, round).await {
        Ok(Ok(response)) if response.tool_calls.is_empty() => {
            Some(response.content).filter(|text| !text.trim().is_empty())
        }
        Ok(Ok(_)) => {
            warn!("Planner: the answer round called a tool instead of answering");
            None
        }
        Ok(Err(e)) => {
            warn!("Planner: the answer round failed: {e}");
            None
        }
        Err(_) => {
            warn!("Planner: the answer round timed out");
            None
        }
    }
}

#[cfg(test)]
// `#[tokio::test]` expands to `Runtime::block_on`, which the crate's lint set
// disallows in library code. Same waiver as the other async test modules here.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn text_answers_a_waiting_requester() {
        assert!(for_requester(async { text_is_final("Here is the analysis.") }).await);
    }

    #[tokio::test]
    async fn an_empty_round_answers_nobody() {
        assert!(!for_requester(async { text_is_final("  \n") }).await);
    }

    #[tokio::test]
    async fn an_autonomous_cycle_is_still_expected_to_act() {
        assert!(!text_is_final("I should look at the colony first."));
    }

    #[tokio::test]
    async fn the_mark_ends_with_the_cycle() {
        for_requester(async {}).await;
        assert!(!text_is_final("Here is the analysis."));
    }
}
