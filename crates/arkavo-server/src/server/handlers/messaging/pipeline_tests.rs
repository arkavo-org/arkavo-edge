//! `message/send` to the roles of a pipeline kit, through the handler
//! itself.
//!
//! The end-to-end tests put two agents behind real JSON-RPC servers on
//! loopback and talk to them the way any client does.

use std::sync::Arc;
use std::time::Duration;

use arkavo_protocol::types::{
    MessagePart, MessageSendRequest, MessageSendResponse, TaskGetRequest, TaskGetResponse,
    TaskStatus,
};
use arkavo_protocol::{A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, HttpTransport};
use arkavo_swarmkit::Manifest;
use serde_json::{Value, json};

use super::test_agent::{Agent, Script, message, purpose, result_text, step_marker, text};
use crate::server::pipeline::{Driving, FAILURE_CODE, FAILURE_SCHEMA, RESULT_SCHEMA, driving_for};

const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "two-step"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "write and review"
roles:
  - id: writer
    role_type: specialist
    agent_provisioning: {}
    handoffs: [{to: reviewer, on: always}]
    context_scope: {can_read: [self], can_write: [self]}
  - id: reviewer
    role_type: critic
    agent_provisioning: {}
    context_scope: {can_read: [writer, self], can_write: [self]}
coordination:
  topology: pipeline
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 100000
    max_cost_usd: 1.0
  network:
    egress_allowed: false
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures: []
"#;

fn manifest() -> Manifest {
    arkavo_swarmkit::parse_yaml(KIT).expect("the fixture kit is valid")
}

fn writer_drives() -> Driving {
    let driving = driving_for(&manifest(), "writer");
    assert!(matches!(driving, Driving::Plan(_)), "{driving:?}");
    driving
}

/// A marked message is a step of someone else's run. Were the entry role to
/// start a run for it, a pipeline whose kit loops back through the entry
/// agent would never end.
#[tokio::test]
async fn a_marked_message_does_not_start_a_pipeline_even_at_the_entry_role() {
    let script = Script::new(vec![text("Reviewed alone.")]);
    let writer = Agent::scripted("writer", &script)
        .await
        .driving(writer_drives());

    let task_id = writer
        .send("Review this draft.", Some(step_marker("run-1")))
        .await;
    let task = writer.finished(task_id).await;

    assert_eq!(task.status, TaskStatus::Completed, "{:?}", task.error);
    assert_eq!(result_text(&task), "Reviewed alone.");
    assert_eq!(script.dispatches(), 1, "one role answered, once");
    assert!(script.prompt(0).contains("Review this draft."));
    assert!(script.prompt(0).contains(&purpose("writer")));
}

/// The key decides, not the shape of what is under it: a sender that meant
/// a step must never be answered with a pipeline.
#[tokio::test]
async fn a_malformed_marker_still_marks_the_message() {
    let script = Script::new(vec![text("Reviewed alone.")]);
    let writer = Agent::scripted("writer", &script)
        .await
        .driving(writer_drives());

    let task_id = writer
        .send("Review this draft.", Some(json!({"pipeline": true})))
        .await;
    let task = writer.finished(task_id).await;

    assert_eq!(result_text(&task), "Reviewed alone.");
    assert_eq!(script.dispatches(), 1);
}

#[tokio::test]
async fn a_role_that_is_not_the_entry_role_does_not_drive() {
    assert_eq!(driving_for(&manifest(), "reviewer"), Driving::No);

    let script = Script::new(vec![text("Reads well.")]);
    let reviewer = Agent::scripted("reviewer", &script)
        .await
        .driving(driving_for(&manifest(), "reviewer"));

    let task_id = reviewer.send("Is this any good?", None).await;
    let task = reviewer.finished(task_id).await;

    assert_eq!(task.status, TaskStatus::Completed, "{:?}", task.error);
    assert_eq!(result_text(&task), "Reads well.");
    assert_eq!(script.dispatches(), 1);
}

#[tokio::test]
async fn an_agent_of_another_topology_does_not_drive() {
    let mut kit = manifest();
    kit.coordination.topology = arkavo_swarmkit::Topology::HubSpoke;
    assert_eq!(driving_for(&kit, "writer"), Driving::No);
}

#[tokio::test]
async fn a_kit_that_cannot_be_read_fails_the_message_with_the_reason() {
    let script = Script::new(vec![text("unused")]);
    let reason = "this agent runs role \"writer\" of a kit that cannot be read".to_string();
    let writer = Agent::scripted("writer", &script)
        .await
        .driving(Driving::Unknown(reason.clone()));

    let task_id = writer.send("Write the launch copy.", None).await;
    let task = writer.finished(task_id).await;

    assert_eq!(task.status, TaskStatus::Failed);
    assert_eq!(task.error.expect("error").message, reason);
    assert_eq!(script.dispatches(), 0, "nothing was answered on a guess");
}

/// A JSON-RPC client, as any caller of an agent is.
struct Client(HttpTransport);

impl Client {
    async fn connect(address: &str) -> Self {
        let transport = HttpTransport::new(arkavo_protocol::TransportConfig {
            max_retries: 0,
            tls_config: arkavo_protocol::transport::TlsConfig {
                require_tls: false,
                ..Default::default()
            },
            ..Default::default()
        })
        .expect("transport");
        transport
            .connect(&A2aEndpoint {
                url: address.to_string(),
                agent_id: "client".to_string(),
                public_key: None,
            })
            .await
            .expect("connect");
        Self(transport)
    }

    async fn call<T: serde::de::DeserializeOwned>(&self, method: &str, params: Value) -> T {
        match self
            .0
            .send_request(A2aRequest::new(method, params))
            .await
            .expect("the agent replied")
        {
            A2aResponse::Success { result, .. } => {
                serde_json::from_value(result).expect("the reply has the method's shape")
            }
            A2aResponse::Error { error, .. } => panic!("{method} failed: {error:?}"),
        }
    }

    async fn send(&self, content: &str) -> String {
        let sent: MessageSendResponse = self
            .call(
                "message/send",
                json!([MessageSendRequest {
                    message: message(content, None),
                    task_id: None,
                }]),
            )
            .await;
        sent.task_id
    }

    /// Poll `tasks/get` until the task ends, collecting the progress
    /// messages seen on the way.
    async fn wait(&self, task_id: &str) -> (TaskGetResponse, Vec<String>) {
        let mut seen = Vec::new();
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let task: TaskGetResponse = self
                    .call(
                        "tasks/get",
                        json!([TaskGetRequest {
                            task_id: task_id.to_string(),
                        }]),
                    )
                    .await;
                if let Some(message) = task.progress.as_ref().and_then(|p| p.message.clone())
                    && seen.last() != Some(&message)
                {
                    seen.push(message);
                }
                if matches!(task.status, TaskStatus::Completed | TaskStatus::Failed) {
                    return (task, seen);
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the task finished")
    }
}

const REQUEST: &str = "Write one line of launch copy for the spring campaign.";
const DRAFT: &str = "Go further for less this spring.";
const REVIEW: &str = "Clear and on brief.";

#[tokio::test(flavor = "multi_thread")]
async fn a_message_to_the_entry_role_runs_the_pipeline_end_to_end() {
    let writer_script = Script::new(vec![text(DRAFT)]);
    let reviewer_script = Script::new(vec![text(REVIEW)]);
    let writer = Arc::new(
        Agent::scripted("writer", &writer_script)
            .await
            .driving(writer_drives()),
    );
    let reviewer = Arc::new(
        Agent::scripted("reviewer", &reviewer_script)
            .await
            .driving(driving_for(&manifest(), "reviewer")),
    );
    // Both agents run an agent loop, as agents started from a kit do. The
    // run has to leave both loops out of it.
    let mut writer_loop = writer.with_agent_loop().await;
    let mut reviewer_loop = reviewer.with_agent_loop().await;
    let (writer_address, writer_server) = writer.serve().await;
    let (reviewer_address, reviewer_server) = reviewer.serve().await;
    writer
        .mesh
        .agent_addresses
        .write()
        .await
        .insert("reviewer".to_string(), reviewer_address);

    let client = Client::connect(&writer_address).await;
    let task_id = client.send(REQUEST).await;
    let (task, progress) = client.wait(&task_id).await;

    assert_eq!(task.status, TaskStatus::Completed, "{:?}", task.error);
    let result = task.result.expect("result");
    let [
        MessagePart::Text { content: text },
        MessagePart::Data { schema, content },
    ] = result.parts.as_slice()
    else {
        panic!("a text part and a data part, got {:?}", result.parts);
    };
    let run_id = content["run_id"].as_str().expect("run id").to_string();
    assert_eq!(
        *text,
        format!(
            "## Final answer (role: reviewer, step 2 of 2)\n\n{REVIEW}\n\n\
             ## Output from role: writer (step 1 of 2)\n\n{DRAFT}\n\n\
             Pipeline run {run_id}"
        )
    );
    assert_eq!(schema, RESULT_SCHEMA);
    assert_eq!(content["status"], "completed");
    assert_eq!(content["final_role"], "reviewer");
    assert_eq!(content["final"], REVIEW);
    assert_eq!(content["roles"], json!(["writer", "reviewer"]));
    assert_eq!(
        content["steps"],
        json!([
            {"step": 1, "role": "writer", "attempt": 1, "superseded": false, "output": DRAFT},
            {"step": 2, "role": "reviewer", "attempt": 1, "superseded": false, "output": REVIEW},
        ])
    );

    // The last thing reported on the caller's task is the last role.
    assert_eq!(
        progress.last().map(String::as_str),
        Some(format!("pipeline run {run_id}: reviewer finished (step 2 of 2)").as_str()),
        "{progress:?}"
    );
    assert_eq!(task.progress.expect("progress").percentage, Some(100));

    // The writer answered the caller's request as it arrived.
    assert_eq!(writer_script.dispatches(), 1);
    assert!(writer_script.prompt(0).contains(REQUEST));
    assert!(writer_script.prompt(0).contains(&purpose("writer")));

    // The reviewer answered one step, holding the request and the draft.
    assert_eq!(reviewer_script.dispatches(), 1);
    let asked = reviewer_script.prompt(0);
    assert!(asked.contains(&purpose("reviewer")), "{asked}");
    assert!(
        asked.contains("## Pipeline step 2 of 2: reviewer"),
        "{asked}"
    );
    assert!(
        asked.contains(&format!("## Original request\n\n{REQUEST}")),
        "{asked}"
    );
    assert!(
        asked.contains(&format!("## Output from role: writer\n\n{DRAFT}")),
        "{asked}"
    );

    // What the reviewer's agent received carried the marker of the run.
    let received = reviewer.store.list_tasks(None).await.expect("tasks");
    assert_eq!(received.len(), 1, "one step, one task");
    let marker = &received[0].message.metadata.as_ref().expect("metadata")["pipeline"];
    assert_eq!(marker["run_id"], run_id);
    assert_eq!(marker["step"], 2);
    assert_eq!(marker["steps"], 2);
    assert_eq!(marker["role"], "reviewer");
    assert_eq!(marker["entry_role"], "writer");

    assert!(writer_loop.try_recv().is_err(), "the run bypasses the loop");
    assert!(
        reviewer_loop.try_recv().is_err(),
        "the step bypasses the loop"
    );
    assert!(
        writer.mesh.pending_delegations.read().await.is_empty(),
        "nothing for the writer's loop to collect later"
    );

    writer_server.stop().expect("writer server was running");
    reviewer_server.stop().expect("reviewer server was running");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_role_that_fails_its_step_fails_the_callers_task_end_to_end() {
    let writer_script = Script::new(vec![text(DRAFT)]);
    let writer = Arc::new(
        Agent::scripted("writer", &writer_script)
            .await
            .driving(writer_drives()),
    );
    // An agent with no router fails every message it is sent.
    let reviewer = Arc::new(Agent::new("reviewer", None).await);
    let (writer_address, writer_server) = writer.serve().await;
    let (reviewer_address, reviewer_server) = reviewer.serve().await;
    writer
        .mesh
        .agent_addresses
        .write()
        .await
        .insert("reviewer".to_string(), reviewer_address);

    let client = Client::connect(&writer_address).await;
    let task_id = client.send(REQUEST).await;
    let (task, _progress) = client.wait(&task_id).await;

    assert_eq!(task.status, TaskStatus::Failed);
    assert!(task.result.is_none(), "a failed run has no result");
    let error = task.error.expect("error");
    assert_eq!(error.code, FAILURE_CODE);
    let details = error.details.expect("details");
    let run_id = details["run_id"].as_str().expect("run id");
    let step_task = reviewer.store.list_tasks(None).await.expect("tasks")[0].id;
    assert_eq!(
        error.message,
        format!(
            "pipeline run {run_id} failed at role \"reviewer\" (step 2 of 2): agent \
             \"reviewer\" failed task {step_task}: agent has no router configured to execute \
             this message"
        )
    );
    assert_eq!(details["schema"], FAILURE_SCHEMA);
    assert_eq!(details["reason"], "step_failed");
    assert_eq!(details["role"], "reviewer");
    assert_eq!(details["step"], 2);
    assert_eq!(
        details["steps"],
        json!([
            {"step": 1, "role": "writer", "attempt": 1, "superseded": false, "output": DRAFT},
        ])
    );

    writer_server.stop().expect("writer server was running");
    reviewer_server.stop().expect("reviewer server was running");
}
