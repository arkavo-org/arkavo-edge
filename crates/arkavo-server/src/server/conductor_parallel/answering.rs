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

/// True when a round that called no tool has already produced the cycle's
/// answer, so the planner must stop instead of asking for a tool call.
pub(super) fn text_is_final(text: &str) -> bool {
    let requester_waiting = REQUESTER_WAITING.try_with(|()| ()).is_ok();
    requester_waiting && !text.trim().is_empty()
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
