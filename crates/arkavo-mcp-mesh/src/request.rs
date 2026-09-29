//! Send a message to a named agent and wait for its answer.
//!
//! `send_task` is for a model that delegates and moves on: it forgives a
//! misspelt agent id, records the delegation so a later cycle collects the
//! result, and hands the receiver a budget for the task. A caller that runs
//! roles in a fixed order needs none of that and is harmed by each part of
//! it. A near match would deliver a role's input to a different agent. A
//! recorded delegation would surface the answer a second time, in the
//! sender's own agent loop, as advice to act on. A default budget would
//! replace the receiver's own with a two-minute one.

use std::time::Duration;

use arkavo_protocol::types::{Message, TaskGetResponse, TaskStatus};
use tokio::time::Instant;

use crate::peer::{Patience, Peer, PeerError, text_of};
use crate::{MeshToolsState, discover_and_register_agents};

/// First wait between two looks at the task. An agent on the same machine
/// often answers a short request within it.
const FIRST_POLL: Duration = Duration::from_millis(200);

/// Longest wait between two looks at the task.
const SLOWEST_POLL: Duration = Duration::from_secs(1);

/// Looks at the task that may fail in a row before the agent is given up
/// on. One failed look is a busy agent; this many is an agent that has gone.
const UNANSWERED_POLLS: u32 = 5;

/// Longest a single call waits for the agent to reply.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// An agent's answer to a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskAnswer {
    /// The task the agent opened for the message.
    pub task_id: String,
    /// Every text part of the result, in full.
    pub text: String,
}

/// Why a message sent with [`send_and_wait`] has no answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RequestError {
    #[error(
        "agent {agent_id:?} was not found; agents known here: {}",
        known_agents(known)
    )]
    AgentNotFound {
        agent_id: String,
        known: Vec<String>,
    },

    #[error("agent {agent_id:?} could not be reached: {reason}")]
    Unreachable { agent_id: String, reason: String },

    #[error("agent {agent_id:?} refused the message: {reason}")]
    Refused { agent_id: String, reason: String },

    #[error("agent {agent_id:?} failed task {task_id}: {message}")]
    Failed {
        agent_id: String,
        task_id: String,
        /// The agent's error code, empty when it gave none.
        code: String,
        message: String,
    },

    #[error("task {task_id} on agent {agent_id:?} was canceled")]
    Canceled { agent_id: String, task_id: String },

    #[error("agent {agent_id:?} rejected task {task_id}")]
    Rejected { agent_id: String, task_id: String },

    #[error(
        "agent {agent_id:?} stopped task {task_id} to wait for {waiting_for}, which nothing here can give it"
    )]
    Stalled {
        agent_id: String,
        task_id: String,
        /// `input` or `authentication`.
        waiting_for: &'static str,
    },

    #[error("agent {agent_id:?} did not answer within {}", seconds(*waited))]
    TimedOut {
        agent_id: String,
        /// The task left running, when the message got as far as opening one.
        task_id: Option<String>,
        waited: Duration,
    },
}

fn known_agents(known: &[String]) -> String {
    if known.is_empty() {
        "none".to_string()
    } else {
        known.join(", ")
    }
}

fn seconds(waited: Duration) -> String {
    if waited < Duration::from_secs(1) {
        format!("{}ms", waited.as_millis())
    } else {
        format!("{}s", waited.as_secs())
    }
}

/// Whether an agent that is not where it was expected may be looked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lookup {
    /// Browse the network for it, once.
    Network,
    /// Use only the addresses already known.
    #[cfg(test)]
    KnownOnly,
}

/// Send `message` to the agent whose id is exactly `agent_id` and wait up to
/// `timeout` for the task it opens to finish.
///
/// The message is sent as given: no metadata is added to it. Nothing is
/// recorded in `state` about the task, so the agent loop never sees it as a
/// delegation of its own. An agent that is not known, or is no longer at the
/// address it was known at, is looked for once on the network before the
/// call gives up.
pub async fn send_and_wait(
    state: &MeshToolsState,
    agent_id: &str,
    message: Message,
    timeout: Duration,
) -> Result<TaskAnswer, RequestError> {
    request(state, agent_id, message, timeout, Lookup::Network).await
}

async fn request(
    state: &MeshToolsState,
    agent_id: &str,
    message: Message,
    timeout: Duration,
    lookup: Lookup,
) -> Result<TaskAnswer, RequestError> {
    let deadline = Instant::now() + timeout;
    let timed_out = |task_id: Option<String>| RequestError::TimedOut {
        agent_id: agent_id.to_string(),
        task_id,
        waited: timeout,
    };

    let opened = tokio::time::timeout_at(
        deadline,
        open_task(state, agent_id, message, deadline, lookup),
    )
    .await
    .map_err(|_| timed_out(None))?;
    let (peer, task_id) = opened?;

    let outcome = wait_for(&peer, agent_id, &task_id, deadline).await;
    peer.close().await;
    match outcome {
        Some(result) => result,
        None => Err(timed_out(Some(task_id))),
    }
}

/// Deliver the message and return the task the agent opened for it.
///
/// An agent that was restarted answers on a new port while this process
/// still holds the old address. When the known address does not answer, the
/// agent is looked for again and the message goes to the address found,
/// provided it is a different one. The first delivery reached nobody, so
/// this is still the only task opened for the message.
async fn open_task(
    state: &MeshToolsState,
    agent_id: &str,
    message: Message,
    deadline: Instant,
    lookup: Lookup,
) -> Result<(Peer, String), RequestError> {
    let (address, fresh) = address_of(state, agent_id, lookup).await?;
    let failed = match deliver(&address, agent_id, message.clone(), deadline).await {
        Ok(opened) => return Ok(opened),
        Err(e) => e,
    };
    if fresh || lookup != Lookup::Network || !matches!(failed, RequestError::Unreachable { .. }) {
        return Err(failed);
    }
    rediscover(state, agent_id).await;
    match state.agent_addresses.read().await.get(agent_id) {
        Some(found) if *found != address => {
            let found = found.clone();
            deliver(&found, agent_id, message, deadline).await
        }
        _ => Err(failed),
    }
}

async fn deliver(
    address: &str,
    agent_id: &str,
    message: Message,
    deadline: Instant,
) -> Result<(Peer, String), RequestError> {
    let peer = Peer::connect(address, agent_id, patience_until(deadline))
        .await
        .map_err(|e| unreachable(agent_id, &e))?;
    match peer.send_message(message).await {
        Ok(response) => Ok((peer, response.task_id)),
        Err(e) => {
            peer.close().await;
            Err(if e.is_answer() {
                RequestError::Refused {
                    agent_id: agent_id.to_string(),
                    reason: e.to_string(),
                }
            } else {
                unreachable(agent_id, &e)
            })
        }
    }
}

/// Poll the task until it ends. `None` when the deadline came first.
async fn wait_for(
    peer: &Peer,
    agent_id: &str,
    task_id: &str,
    deadline: Instant,
) -> Option<Result<TaskAnswer, RequestError>> {
    let mut pause = FIRST_POLL;
    let mut unanswered = 0;
    loop {
        match tokio::time::timeout_at(deadline, peer.get_task(task_id)).await {
            Err(_) => return None,
            Ok(Ok(task)) => {
                unanswered = 0;
                if let Some(ended) = ending(agent_id, task) {
                    return Some(ended);
                }
            }
            Ok(Err(e)) => {
                unanswered += 1;
                if unanswered >= UNANSWERED_POLLS {
                    return Some(Err(unreachable(agent_id, &e)));
                }
            }
        }
        if Instant::now() + pause >= deadline {
            tokio::time::sleep_until(deadline).await;
            return None;
        }
        tokio::time::sleep(pause).await;
        pause = (pause * 2).min(SLOWEST_POLL);
    }
}

/// What a task's state means for the caller, or `None` while it is running.
fn ending(agent_id: &str, task: TaskGetResponse) -> Option<Result<TaskAnswer, RequestError>> {
    let agent_id = agent_id.to_string();
    let task_id = task.task_id;
    Some(match task.status {
        TaskStatus::Submitted | TaskStatus::Working => return None,
        TaskStatus::Completed => Ok(TaskAnswer {
            task_id,
            text: task.result.as_ref().map(text_of).unwrap_or_default(),
        }),
        TaskStatus::Failed => {
            let (code, message) = task.error.map_or_else(
                || (String::new(), "the agent gave no reason".to_string()),
                |e| (e.code, e.message),
            );
            Err(RequestError::Failed {
                agent_id,
                task_id,
                code,
                message,
            })
        }
        TaskStatus::Canceled => Err(RequestError::Canceled { agent_id, task_id }),
        TaskStatus::Rejected => Err(RequestError::Rejected { agent_id, task_id }),
        TaskStatus::InputRequired => Err(RequestError::Stalled {
            agent_id,
            task_id,
            waiting_for: "input",
        }),
        TaskStatus::AuthRequired => Err(RequestError::Stalled {
            agent_id,
            task_id,
            waiting_for: "authentication",
        }),
    })
}

/// The address of the agent named exactly `agent_id`, and whether it was
/// found by a lookup made just now.
async fn address_of(
    state: &MeshToolsState,
    agent_id: &str,
    lookup: Lookup,
) -> Result<(String, bool), RequestError> {
    if let Some(address) = state.agent_addresses.read().await.get(agent_id) {
        return Ok((address.clone(), false));
    }
    if lookup == Lookup::Network {
        tracing::info!("Agent '{agent_id}' is not known yet, looking for it on the network");
        rediscover(state, agent_id).await;
    }
    let (found, mut known) = {
        let addresses = state.agent_addresses.read().await;
        (
            addresses.get(agent_id).cloned(),
            addresses.keys().cloned().collect::<Vec<_>>(),
        )
    };
    match found {
        Some(address) => Ok((address, true)),
        None => {
            known.sort();
            Err(RequestError::AgentNotFound {
                agent_id: agent_id.to_string(),
                known,
            })
        }
    }
}

async fn rediscover(state: &MeshToolsState, agent_id: &str) {
    if let Err(e) = discover_and_register_agents(state).await {
        tracing::debug!("Discovery failed while looking for '{agent_id}': {e}");
    }
}

/// A single call never outlasts the caller's deadline, and never waits
/// longer than [`CALL_TIMEOUT`] however far off the deadline is.
fn patience_until(deadline: Instant) -> Patience {
    let left = deadline.saturating_duration_since(Instant::now());
    let timeout = left.clamp(Duration::from_millis(1), CALL_TIMEOUT);
    Patience {
        timeout_ms: u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        // The message opens a task each time it is delivered, so a retry
        // after a lost reply would run the role twice.
        max_retries: 0,
    }
}

fn unreachable(agent_id: &str, error: &PeerError) -> RequestError {
    RequestError::Unreachable {
        agent_id: agent_id.to_string(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#[allow(clippy::disallowed_methods)]
mod tests;
