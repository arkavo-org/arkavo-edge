//! `send_and_wait` against an agent that is a script: a JSON-RPC server on
//! loopback that answers `message/send` and `tasks/get` as each test says.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arkavo_protocol::types::{
    Message, MessagePart, MessageSendRequest, MessageSendResponse, TaskError, TaskGetRequest,
    TaskGetResponse, TaskStatus,
};
use jsonrpsee::RpcModule;
use jsonrpsee::server::{Server, ServerHandle};
use jsonrpsee::types::ErrorObjectOwned;
use serde_json::json;

use super::{Lookup, RequestError, TaskAnswer, request};
use crate::MeshToolsState;

const TASK_ID: &str = "7f1f4a2e-6f0e-4c3b-9d67-2f6c1f0a9b11";

/// What the scripted agent does with a message.
enum OnSend {
    Accept,
    Refuse(i32, &'static str),
}

struct Script {
    on_send: OnSend,
    /// Answers to `tasks/get`, in order. The last one repeats.
    states: Mutex<VecDeque<TaskGetResponse>>,
    received: Mutex<Vec<Message>>,
    polls: Mutex<usize>,
}

struct ScriptedAgent {
    address: String,
    script: Arc<Script>,
    handle: ServerHandle,
}

impl ScriptedAgent {
    async fn start(on_send: OnSend, states: Vec<TaskGetResponse>) -> Self {
        let script = Arc::new(Script {
            on_send,
            states: Mutex::new(states.into()),
            received: Mutex::new(Vec::new()),
            polls: Mutex::new(0),
        });
        let mut module = RpcModule::new(script.clone());
        module
            .register_method("message/send", |params, script, _| {
                let request: MessageSendRequest = params.one()?;
                script
                    .received
                    .lock()
                    .expect("received")
                    .push(request.message);
                match script.on_send {
                    OnSend::Accept => Ok(MessageSendResponse {
                        task_id: TASK_ID.to_string(),
                        status: TaskStatus::Submitted,
                        response: None,
                    }),
                    OnSend::Refuse(code, message) => {
                        Err(ErrorObjectOwned::owned(code, message, None::<()>))
                    }
                }
            })
            .expect("message/send registers");
        module
            .register_method("tasks/get", |params, script, _| {
                let request: TaskGetRequest = params.one()?;
                assert_eq!(request.task_id, TASK_ID, "polls the task it was given");
                *script.polls.lock().expect("polls") += 1;
                let mut states = script.states.lock().expect("states");
                let state = if states.len() > 1 {
                    states.pop_front()
                } else {
                    states.front().cloned()
                };
                state.ok_or_else(|| ErrorObjectOwned::owned(-32602, "Task not found", None::<()>))
            })
            .expect("tasks/get registers");

        let server = Server::builder()
            .build("127.0.0.1:0")
            .await
            .expect("loopback server");
        let address = format!("http://{}", server.local_addr().expect("bound address"));
        Self {
            address,
            script,
            handle: server.start(module),
        }
    }

    fn received(&self) -> Vec<Message> {
        self.script.received.lock().expect("received").clone()
    }

    fn polls(&self) -> usize {
        *self.script.polls.lock().expect("polls")
    }

    async fn stop(self) {
        self.handle.stop().expect("server was running");
        self.handle.stopped().await;
    }
}

fn state(status: TaskStatus) -> TaskGetResponse {
    TaskGetResponse {
        task_id: TASK_ID.to_string(),
        status,
        result: None,
        error: None,
        progress: None,
    }
}

fn completed(parts: Vec<MessagePart>) -> TaskGetResponse {
    TaskGetResponse {
        result: Some(Message {
            parts,
            metadata: None,
        }),
        ..state(TaskStatus::Completed)
    }
}

fn text(content: &str) -> MessagePart {
    MessagePart::Text {
        content: content.to_string(),
    }
}

fn message(content: &str, metadata: Option<serde_json::Value>) -> Message {
    Message {
        parts: vec![text(content)],
        metadata,
    }
}

async fn mesh_knowing(agents: &[(&str, &str)]) -> MeshToolsState {
    let mesh = MeshToolsState::new();
    for (agent_id, address) in agents {
        mesh.agent_addresses
            .write()
            .await
            .insert((*agent_id).to_string(), (*address).to_string());
    }
    mesh
}

async fn ask(
    mesh: &MeshToolsState,
    agent_id: &str,
    message: Message,
    timeout: Duration,
) -> Result<TaskAnswer, RequestError> {
    request(mesh, agent_id, message, timeout, Lookup::KnownOnly).await
}

const PATIENT: Duration = Duration::from_secs(20);

#[tokio::test]
async fn the_answer_is_the_full_text_of_the_finished_task() {
    let long = "The brief, in full. ".repeat(20_000);
    let agent = ScriptedAgent::start(
        OnSend::Accept,
        vec![completed(vec![
            text(&long),
            MessagePart::Data {
                schema: "urn:example:scores".to_string(),
                content: json!({"quality": 0.9}),
            },
            text("VERDICT: PASS"),
        ])],
    )
    .await;
    let mesh = mesh_knowing(&[("copy", &agent.address)]).await;

    let answer = ask(&mesh, "copy", message("Write the brief.", None), PATIENT)
        .await
        .expect("the task finished");

    assert_eq!(answer.task_id, TASK_ID);
    assert_eq!(answer.text.len(), long.len() + "\nVERDICT: PASS".len());
    assert_eq!(answer.text, format!("{long}\nVERDICT: PASS"));
    agent.stop().await;
}

#[tokio::test]
async fn a_running_task_is_waited_for() {
    let agent = ScriptedAgent::start(
        OnSend::Accept,
        vec![
            state(TaskStatus::Submitted),
            state(TaskStatus::Working),
            state(TaskStatus::Working),
            completed(vec![text("done")]),
        ],
    )
    .await;
    let mesh = mesh_knowing(&[("copy", &agent.address)]).await;

    let answer = ask(&mesh, "copy", message("Write.", None), PATIENT)
        .await
        .expect("the task finished");

    assert_eq!(answer.text, "done");
    assert_eq!(agent.polls(), 4);
    agent.stop().await;
}

/// A pipeline must never deliver a role's input to a similarly named agent,
/// which is what `send_task` does for ids within two edits of a known one.
#[tokio::test]
async fn only_the_exact_agent_id_is_a_match() {
    let agent = ScriptedAgent::start(OnSend::Accept, vec![completed(vec![text("done")])]).await;
    let mesh = mesh_knowing(&[
        ("critics", &agent.address),
        ("critic-2", &agent.address),
        ("Critic", &agent.address),
    ])
    .await;

    let outcome = ask(&mesh, "critic", message("Review.", None), PATIENT).await;

    assert_eq!(
        outcome,
        Err(RequestError::AgentNotFound {
            agent_id: "critic".to_string(),
            known: vec![
                "Critic".to_string(),
                "critic-2".to_string(),
                "critics".to_string()
            ],
        })
    );
    assert!(agent.received().is_empty(), "nothing was delivered");
    agent.stop().await;
}

#[tokio::test]
async fn an_unknown_agent_is_reported_with_the_agents_that_are_known() {
    let mesh = mesh_knowing(&[]).await;
    let err = ask(&mesh, "critic", message("Review.", None), PATIENT)
        .await
        .expect_err("nobody to ask");
    assert_eq!(
        err.to_string(),
        "agent \"critic\" was not found; agents known here: none"
    );
}

/// Regression guard for the two things `send_task` adds that a waiting
/// caller must not: a recorded delegation, which the agent loop would turn
/// into a cycle of its own, and a default budget allocation, which replaces
/// the receiver's budget window with a 120 s one.
#[tokio::test]
async fn nothing_is_recorded_and_nothing_is_added_to_the_message() {
    let agent = ScriptedAgent::start(OnSend::Accept, vec![completed(vec![text("done")])]).await;
    let mesh = mesh_knowing(&[("copy", &agent.address)]).await;
    let marked = json!({"pipeline": {"run_id": "r-1", "step": 2}});

    ask(
        &mesh,
        "copy",
        message("Write.", Some(marked.clone())),
        PATIENT,
    )
    .await
    .expect("the task finished");
    ask(&mesh, "copy", message("Write again.", None), PATIENT)
        .await
        .expect("the task finished");

    assert!(mesh.pending_delegations.read().await.is_empty());
    assert!(mesh.collect_completed().await.is_empty());
    let received = agent.received();
    assert_eq!(received.len(), 2);
    assert_eq!(received[0].metadata, Some(marked));
    assert_eq!(received[1].metadata, None);
    agent.stop().await;
}

#[tokio::test]
async fn a_failed_task_carries_the_agents_reason() {
    let agent = ScriptedAgent::start(
        OnSend::Accept,
        vec![TaskGetResponse {
            error: Some(TaskError {
                code: "AGENT_CYCLE_FAILED".to_string(),
                message: "compute budget exhausted".to_string(),
                details: None,
            }),
            ..state(TaskStatus::Failed)
        }],
    )
    .await;
    let mesh = mesh_knowing(&[("copy", &agent.address)]).await;

    let err = ask(&mesh, "copy", message("Write.", None), PATIENT)
        .await
        .expect_err("the task failed");

    assert_eq!(
        err,
        RequestError::Failed {
            agent_id: "copy".to_string(),
            task_id: TASK_ID.to_string(),
            code: "AGENT_CYCLE_FAILED".to_string(),
            message: "compute budget exhausted".to_string(),
        }
    );
    assert_eq!(
        err.to_string(),
        format!("agent \"copy\" failed task {TASK_ID}: compute budget exhausted")
    );
    agent.stop().await;
}

#[tokio::test]
async fn a_failure_without_a_reason_says_so() {
    let agent = ScriptedAgent::start(OnSend::Accept, vec![state(TaskStatus::Failed)]).await;
    let mesh = mesh_knowing(&[("copy", &agent.address)]).await;

    let err = ask(&mesh, "copy", message("Write.", None), PATIENT)
        .await
        .expect_err("the task failed");

    assert!(
        matches!(&err, RequestError::Failed { code, message, .. }
            if code.is_empty() && message == "the agent gave no reason"),
        "{err:?}"
    );
    agent.stop().await;
}

#[tokio::test]
async fn canceled_rejected_and_stalled_tasks_are_told_apart() {
    for (status, expected) in [
        (
            TaskStatus::Canceled,
            RequestError::Canceled {
                agent_id: "copy".to_string(),
                task_id: TASK_ID.to_string(),
            },
        ),
        (
            TaskStatus::Rejected,
            RequestError::Rejected {
                agent_id: "copy".to_string(),
                task_id: TASK_ID.to_string(),
            },
        ),
        (
            TaskStatus::InputRequired,
            RequestError::Stalled {
                agent_id: "copy".to_string(),
                task_id: TASK_ID.to_string(),
                waiting_for: "input",
            },
        ),
        (
            TaskStatus::AuthRequired,
            RequestError::Stalled {
                agent_id: "copy".to_string(),
                task_id: TASK_ID.to_string(),
                waiting_for: "authentication",
            },
        ),
    ] {
        let agent = ScriptedAgent::start(OnSend::Accept, vec![state(status)]).await;
        let mesh = mesh_knowing(&[("copy", &agent.address)]).await;

        let outcome = ask(&mesh, "copy", message("Write.", None), PATIENT).await;

        assert_eq!(outcome, Err(expected), "{status:?}");
        agent.stop().await;
    }
}

#[tokio::test]
async fn a_task_that_does_not_finish_in_time_is_a_timeout_naming_the_task() {
    let agent = ScriptedAgent::start(OnSend::Accept, vec![state(TaskStatus::Working)]).await;
    let mesh = mesh_knowing(&[("copy", &agent.address)]).await;
    let timeout = Duration::from_millis(700);

    let started = std::time::Instant::now();
    let err = ask(&mesh, "copy", message("Write.", None), timeout)
        .await
        .expect_err("the task never finished");
    let waited = started.elapsed();

    assert_eq!(
        err,
        RequestError::TimedOut {
            agent_id: "copy".to_string(),
            task_id: Some(TASK_ID.to_string()),
            waited: timeout,
        }
    );
    assert_eq!(
        err.to_string(),
        "agent \"copy\" did not answer within 700ms"
    );
    assert!(waited >= timeout, "returned after {waited:?}");
    assert!(
        waited < timeout + Duration::from_secs(5),
        "returned after {waited:?}"
    );
    assert!(agent.polls() >= 1, "the task was looked at");
    agent.stop().await;
}

#[tokio::test]
async fn a_refused_message_carries_the_refusal() {
    let agent = ScriptedAgent::start(
        OnSend::Refuse(-32002, "Session budget exhausted"),
        vec![state(TaskStatus::Working)],
    )
    .await;
    let mesh = mesh_knowing(&[("copy", &agent.address)]).await;

    let err = ask(&mesh, "copy", message("Write.", None), PATIENT)
        .await
        .expect_err("the message was refused");

    assert_eq!(
        err,
        RequestError::Refused {
            agent_id: "copy".to_string(),
            reason: "-32002: Session budget exhausted".to_string(),
        }
    );
    assert_eq!(agent.polls(), 0, "no task to look at");
    agent.stop().await;
}

/// An address nothing listens on: bound once to learn a free port, then
/// released.
async fn vacant_address() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback port");
    let address = format!("http://{}", listener.local_addr().expect("bound address"));
    drop(listener);
    address
}

#[tokio::test]
async fn an_agent_that_is_not_listening_is_unreachable() {
    let mesh = mesh_knowing(&[("copy", &vacant_address().await)]).await;

    let err = ask(&mesh, "copy", message("Write.", None), PATIENT)
        .await
        .expect_err("nobody is listening");

    assert!(
        matches!(&err, RequestError::Unreachable { agent_id, .. } if agent_id == "copy"),
        "{err:?}"
    );
}

/// An agent that stops answering is given up on well before a long
/// deadline, so its caller can report the fault instead of waiting it out.
#[tokio::test]
async fn an_agent_that_goes_away_mid_task_is_unreachable() {
    let agent = ScriptedAgent::start(OnSend::Accept, vec![state(TaskStatus::Working)]).await;
    let mesh = mesh_knowing(&[("copy", &agent.address)]).await;

    let waiting = tokio::spawn(async move {
        ask(
            &mesh,
            "copy",
            message("Write.", None),
            Duration::from_secs(120),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(20), async {
        while agent.polls() == 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the task was looked at");
    agent.stop().await;

    let err = tokio::time::timeout(Duration::from_secs(60), waiting)
        .await
        .expect("gave up before the deadline")
        .expect("no panic")
        .expect_err("the agent is gone");
    assert!(
        matches!(&err, RequestError::Unreachable { agent_id, .. } if agent_id == "copy"),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_finished_task_with_no_text_is_an_empty_answer() {
    let agent = ScriptedAgent::start(OnSend::Accept, vec![state(TaskStatus::Completed)]).await;
    let mesh = mesh_knowing(&[("copy", &agent.address)]).await;

    let answer = ask(&mesh, "copy", message("Write.", None), PATIENT)
        .await
        .expect("the task finished");

    assert_eq!(answer.text, "");
    agent.stop().await;
}
