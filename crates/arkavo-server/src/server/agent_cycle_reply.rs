//! Answering the requester of an agent cycle.
//!
//! A cycle serving an A2A request has three possible endings — assistant text,
//! a summary of the cycle's tool activity, or an explicit failure — and every
//! one of them has to reach the requester. Silence is not an ending: the caller
//! polls `tasks/get` until its own timeout expires and then reports nothing.
//!
//! This module owns both halves of that contract: the agent loop's side (drain
//! requests into a cycle, answer them when it ends) and the A2A handler's side
//! (turn an answer into a terminal task state the poller can read).

use super::agent_event::{CycleId, CycleOutcome, CycleReceipt, MessageDisposition, PendingMessage};
use arkavo_protocol::types::{Message, MessagePart, TaskError};
use arkavo_tasks::task_executor::TaskExecutor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{info, warn};

/// Ceiling on how long a requester's task is left non-terminal.
///
/// The A2A chat client polls for five minutes before giving up, so the server
/// must reach a terminal state before that or the caller still sees silence.
pub(super) const REQUEST_REPLY_BUDGET: Duration = Duration::from_secs(240);

/// Requests folded into one cycle: their prompt text, their receipts, and the
/// channels still owed an answer.
pub(super) struct DrainedMessages {
    pub block: String,
    pub receipts: Vec<(oneshot::Sender<CycleReceipt>, CycleReceipt)>,
    pub waiters: Vec<oneshot::Sender<CycleOutcome>>,
}

/// Move every queued request into the cycle identified by `cycle_id`.
pub(super) fn drain_pending_messages(
    pending: &mut Vec<PendingMessage>,
    cycle_id: CycleId,
) -> DrainedMessages {
    let mut block = String::new();
    let mut receipts = Vec::new();
    let mut waiters = Vec::new();
    for mut msg in pending.drain(..) {
        if !block.is_empty() {
            block.push('\n');
        }
        block.push_str(&msg.content);
        if let Some(reply) = msg.reply.take() {
            receipts.push((
                reply,
                CycleReceipt {
                    cycle_id,
                    correlation_id: msg.correlation_id,
                    disposition: MessageDisposition::Incorporated { cycle_id },
                },
            ));
        }
        if let Some(outcome) = msg.outcome.take() {
            waiters.push(outcome);
        }
    }
    DrainedMessages {
        block,
        receipts,
        waiters,
    }
}

/// Answer every request the cycle was serving.
///
/// A dropped receiver (requester already gone) is not an error, so send
/// failures are ignored — but each waiter is always attempted exactly once.
pub(super) fn answer_waiters(waiters: Vec<oneshot::Sender<CycleOutcome>>, outcome: &CycleOutcome) {
    if waiters.is_empty() {
        return;
    }
    match outcome {
        CycleOutcome::Completed { text } => info!(
            requesters = waiters.len(),
            chars = text.len(),
            "Answering cycle requesters"
        ),
        CycleOutcome::Failed { error } => warn!(
            requesters = waiters.len(),
            error = %error,
            "Failing cycle requesters"
        ),
    }
    for waiter in waiters {
        let _ = waiter.send(outcome.clone());
    }
}

/// Turn a cycle's conductor result into the answer its requesters receive.
///
/// An empty result means the cycle produced neither text nor tool activity to
/// summarise. That is a failure from the requester's point of view, and saying
/// so beats echoing an unrelated summary of earlier cycles.
pub(super) fn outcome_for_cycle(result: &Result<String, String>) -> CycleOutcome {
    match result {
        Ok(text) if !text.trim().is_empty() => CycleOutcome::Completed { text: text.clone() },
        Ok(_) => CycleOutcome::Failed {
            error: "agent cycle produced no output".to_string(),
        },
        Err(error) => CycleOutcome::Failed {
            error: error.clone(),
        },
    }
}

/// Refuse requests that are still queued when the cycle meant to serve them
/// cannot run at all (no purpose configured, compute budget gone, shutdown).
pub(super) fn reject_pending(pending: &mut Vec<PendingMessage>, cycle_id: CycleId, reason: &str) {
    if pending.is_empty() {
        return;
    }
    let drained = drain_pending_messages(pending, cycle_id);
    for (sender, mut receipt) in drained.receipts {
        receipt.disposition = MessageDisposition::Rejected {
            reason: reason.to_string(),
        };
        let _ = sender.send(receipt);
    }
    answer_waiters(
        drained.waiters,
        &CycleOutcome::Failed {
            error: reason.to_string(),
        },
    );
}

/// Wait for the cycle serving `task_id` and put the task into a terminal state.
///
/// Runs on the A2A handler side. Once this returns, no other writer touches the
/// task: the agent loop only holds the answer channels, whose sends fail
/// harmlessly after the wait is over. That makes the terminal state final even
/// when a wedged cycle finishes late.
pub(super) async fn deliver_outcome_to_task(
    task_executor: Arc<TaskExecutor>,
    task_id: uuid::Uuid,
    receipt_rx: oneshot::Receiver<CycleReceipt>,
    outcome_rx: oneshot::Receiver<CycleOutcome>,
    budget: Duration,
) {
    let deadline = tokio::time::Instant::now() + budget;

    match tokio::time::timeout_at(deadline, receipt_rx).await {
        Ok(Ok(receipt)) => info!(
            correlation_id = %receipt.correlation_id.0,
            cycle = receipt.cycle_id.0,
            "Message incorporated into orchestrator cycle"
        ),
        Ok(Err(_canceled)) => {
            apply_outcome(
                &task_executor,
                &task_id,
                &CycleOutcome::Failed {
                    error: "agent loop dropped the message before any cycle ran".to_string(),
                },
            )
            .await;
            return;
        }
        Err(_timeout) => {
            apply_outcome(
                &task_executor,
                &task_id,
                &CycleOutcome::Failed {
                    error: format!(
                        "agent did not start a cycle for this message within {}s",
                        budget.as_secs()
                    ),
                },
            )
            .await;
            return;
        }
    }

    let outcome = match tokio::time::timeout_at(deadline, outcome_rx).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_canceled)) => CycleOutcome::Failed {
            error: "agent loop ended without answering this message".to_string(),
        },
        Err(_timeout) => CycleOutcome::Failed {
            error: format!("agent cycle did not finish within {}s", budget.as_secs()),
        },
    };
    apply_outcome(&task_executor, &task_id, &outcome).await;
}

/// Write an answer onto the A2A task the requester is polling.
pub(super) async fn apply_outcome(
    task_executor: &Arc<TaskExecutor>,
    task_id: &uuid::Uuid,
    outcome: &CycleOutcome,
) {
    match outcome {
        CycleOutcome::Completed { text } => {
            let message = Message {
                parts: vec![MessagePart::Text {
                    content: text.clone(),
                }],
                metadata: None,
            };
            let result = serde_json::to_value(&message).unwrap_or(serde_json::Value::Null);
            match task_executor.complete_task(task_id, result).await {
                Ok(()) => info!("Task {task_id} completed from orchestrator cycle"),
                Err(e) => warn!("Failed to complete task {task_id}: {e}"),
            }
        }
        CycleOutcome::Failed { error } => {
            let task_error = TaskError {
                code: "AGENT_CYCLE_FAILED".to_string(),
                message: error.clone(),
                details: None,
            };
            match task_executor.fail_task(task_id, task_error).await {
                Ok(()) => warn!("Task {task_id} failed from orchestrator cycle: {error}"),
                Err(e) => warn!("Failed to mark task {task_id} as failed: {e}"),
            }
        }
    }
}

#[cfg(test)]
// `#[tokio::test]` expands to `Runtime::block_on`, which the crate's lint set
// disallows in library code. Same waiver as the other async test modules here.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::server::agent_event::{CorrelationId, MessagePriority};
    use arkavo_protocol::types::TaskStatus;
    use arkavo_tasks::task_executor::TaskExecutorConfig;
    use arkavo_tasks::task_store::{SqliteTaskStore, TaskStore};

    fn request(content: &str) -> (PendingMessage, oneshot::Receiver<CycleOutcome>) {
        let (receipt_tx, _receipt_rx) = oneshot::channel();
        let (outcome_tx, outcome_rx) = oneshot::channel();
        (
            PendingMessage {
                content: content.to_string(),
                task_id: None,
                correlation_id: CorrelationId(uuid::Uuid::new_v4()),
                reply: Some(receipt_tx),
                outcome: Some(outcome_tx),
                priority: MessagePriority::Normal,
            },
            outcome_rx,
        )
    }

    async fn executor_with_task() -> (Arc<TaskExecutor>, Arc<dyn TaskStore>, uuid::Uuid) {
        let store: Arc<dyn TaskStore> = Arc::new(
            SqliteTaskStore::new_in_memory()
                .await
                .expect("in-memory task store"),
        );
        let executor = Arc::new(TaskExecutor::new(
            store.clone(),
            TaskExecutorConfig::default(),
        ));
        let task_id = executor
            .submit_task(Message {
                parts: vec![MessagePart::Text {
                    content: "hello".to_string(),
                }],
                metadata: None,
            })
            .await
            .expect("submit");
        (executor, store, task_id)
    }

    /// Bug 2: a tool-only cycle produced no assistant text and the requester
    /// was never answered. Every drained request must be answered exactly once.
    #[tokio::test]
    async fn every_drained_request_is_answered_once() {
        let (msg_a, mut rx_a) = request("first");
        let (msg_b, mut rx_b) = request("second");
        let mut pending = vec![msg_a, msg_b];

        let drained = drain_pending_messages(&mut pending, CycleId(7));
        assert_eq!(drained.block, "first\nsecond");
        assert_eq!(drained.waiters.len(), 2);
        assert!(
            rx_a.try_recv().is_err(),
            "not answered before the cycle ends"
        );

        answer_waiters(
            drained.waiters,
            &outcome_for_cycle(&Ok("Completed 1 tool call(s). Last result: ok".to_string())),
        );

        let expected = CycleOutcome::Completed {
            text: "Completed 1 tool call(s). Last result: ok".to_string(),
        };
        assert_eq!(rx_a.try_recv().expect("answered"), expected);
        assert_eq!(rx_b.try_recv().expect("answered"), expected);
    }

    /// Bug 1: a routing or budget refusal must reach the requester verbatim.
    #[test]
    fn cycle_failure_carries_the_underlying_message() {
        let refusal = "Budget exceeded: shared budget cannot fund router".to_string();
        assert_eq!(
            outcome_for_cycle(&Err(refusal.clone())),
            CycleOutcome::Failed { error: refusal }
        );
    }

    /// A cycle with neither text nor tool activity is still an explicit answer.
    #[test]
    fn empty_cycle_result_is_an_explicit_failure() {
        assert_eq!(
            outcome_for_cycle(&Ok("   ".to_string())),
            CycleOutcome::Failed {
                error: "agent cycle produced no output".to_string()
            }
        );
    }

    #[tokio::test]
    async fn rejected_requests_are_answered_with_the_reason() {
        let (msg, mut rx) = request("queued");
        let mut pending = vec![msg];
        reject_pending(&mut pending, CycleId(3), "compute budget exhausted");
        assert!(pending.is_empty());
        assert_eq!(
            rx.try_recv().expect("answered"),
            CycleOutcome::Failed {
                error: "compute budget exhausted".to_string()
            }
        );
    }

    /// Bug 2 end state: the requester polls the task, so a finished cycle has
    /// to leave that task Completed with the cycle's text.
    #[tokio::test]
    async fn completed_outcome_completes_the_polled_task() {
        let (executor, store, task_id) = executor_with_task().await;
        let (receipt_tx, receipt_rx) = oneshot::channel();
        let (outcome_tx, outcome_rx) = oneshot::channel();

        receipt_tx
            .send(CycleReceipt {
                cycle_id: CycleId(1),
                correlation_id: CorrelationId(uuid::Uuid::new_v4()),
                disposition: MessageDisposition::Incorporated {
                    cycle_id: CycleId(1),
                },
            })
            .expect("receipt");
        outcome_tx
            .send(CycleOutcome::Completed {
                text: "Completed 1 tool call(s). Last result: 2 agents".to_string(),
            })
            .expect("outcome");

        deliver_outcome_to_task(
            executor,
            task_id,
            receipt_rx,
            outcome_rx,
            Duration::from_secs(5),
        )
        .await;

        let task = store.get_task(&task_id).await.expect("get").expect("task");
        assert_eq!(task.status, TaskStatus::Completed);
        let result = serde_json::to_string(&task.result.expect("result")).expect("json");
        assert!(
            result.contains("2 agents"),
            "carries the cycle text: {result}"
        );
    }

    /// Bug 1 end state: the refusal reaches the poller as a failed task whose
    /// error names the underlying condition.
    #[tokio::test]
    async fn failed_outcome_fails_the_polled_task_with_the_reason() {
        let (executor, store, task_id) = executor_with_task().await;
        let (receipt_tx, receipt_rx) = oneshot::channel();
        let (outcome_tx, outcome_rx) = oneshot::channel();

        receipt_tx
            .send(CycleReceipt {
                cycle_id: CycleId(1),
                correlation_id: CorrelationId(uuid::Uuid::new_v4()),
                disposition: MessageDisposition::Incorporated {
                    cycle_id: CycleId(1),
                },
            })
            .expect("receipt");
        outcome_tx
            .send(CycleOutcome::Failed {
                error: "Budget exceeded: shared budget cannot fund router".to_string(),
            })
            .expect("outcome");

        deliver_outcome_to_task(
            executor,
            task_id,
            receipt_rx,
            outcome_rx,
            Duration::from_secs(5),
        )
        .await;

        let task = store.get_task(&task_id).await.expect("get").expect("task");
        assert_eq!(task.status, TaskStatus::Failed);
        let error = task.error.expect("error");
        assert_eq!(error.code, "AGENT_CYCLE_FAILED");
        assert_eq!(
            error.message,
            "Budget exceeded: shared budget cannot fund router"
        );
    }

    /// A wedged agent loop must still produce a terminal state before the
    /// caller's own timeout, or the caller is back to waiting on silence.
    #[tokio::test]
    async fn a_silent_cycle_fails_the_task_within_the_budget() {
        let (executor, store, task_id) = executor_with_task().await;
        let (receipt_tx, receipt_rx) = oneshot::channel();
        let (outcome_tx, outcome_rx) = oneshot::channel();
        receipt_tx
            .send(CycleReceipt {
                cycle_id: CycleId(1),
                correlation_id: CorrelationId(uuid::Uuid::new_v4()),
                disposition: MessageDisposition::Incorporated {
                    cycle_id: CycleId(1),
                },
            })
            .expect("receipt");

        deliver_outcome_to_task(
            executor,
            task_id,
            receipt_rx,
            outcome_rx,
            Duration::from_millis(50),
        )
        .await;
        drop(outcome_tx);

        let task = store.get_task(&task_id).await.expect("get").expect("task");
        assert_eq!(task.status, TaskStatus::Failed);
        assert!(
            task.error
                .expect("error")
                .message
                .contains("did not finish"),
            "names the stall"
        );
    }

    /// An agent loop that never picks the message up is also a terminal state.
    #[tokio::test]
    async fn a_dropped_message_fails_the_task() {
        let (executor, store, task_id) = executor_with_task().await;
        let (receipt_tx, receipt_rx) = oneshot::channel();
        let (_outcome_tx, outcome_rx) = oneshot::channel();
        drop(receipt_tx);

        deliver_outcome_to_task(
            executor,
            task_id,
            receipt_rx,
            outcome_rx,
            Duration::from_secs(5),
        )
        .await;

        let task = store.get_task(&task_id).await.expect("get").expect("task");
        assert_eq!(task.status, TaskStatus::Failed);
        assert!(task.error.expect("error").message.contains("dropped"));
    }
}
