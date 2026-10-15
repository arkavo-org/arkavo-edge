//! Integration tests for the mesh orchestration tools, driven through their
//! public `Tool` interface against a scripted A2A agent: a JSON-RPC server
//! on loopback that answers `message/send` and `tasks/get` as each test says.

// Every test here is a `#[tokio::test]`, and that macro expands to
// `Runtime::block_on`, which `.clippy.toml` disallows outside test code. An
// integration test file has no `mod tests` to hang the narrower attribute on.
#![allow(clippy::disallowed_methods)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use arkavo_mcp_mesh::{
    AgentQueryTool, GetTaskStatusTool, ListAgentsTool, MeshToolError, MeshToolsState, SendTaskTool,
    Tool, ToolRegistry, create_mesh_tools, register_tools,
};
use arkavo_protocol::types::{
    Message, MessagePart, MessageSendRequest, MessageSendResponse, TaskGetRequest, TaskGetResponse,
    TaskProgress, TaskStatus,
};
use arkavo_test_macros::spec;
use jsonrpsee::RpcModule;
use jsonrpsee::server::{Server, ServerHandle};
use jsonrpsee::types::ErrorObjectOwned;
use serde_json::{Value, json};

/// What the scripted agent answers to `tasks/get`.
enum OnGet {
    Task(TaskGetResponse),
    Error(i32, &'static str),
}

struct Shared {
    on_get: OnGet,
    received: Mutex<Vec<Message>>,
}

struct ScriptedAgent {
    address: String,
    shared: Arc<Shared>,
    handle: ServerHandle,
}

impl ScriptedAgent {
    async fn start(task_id: &str, on_get: OnGet) -> Self {
        let shared = Arc::new(Shared {
            on_get,
            received: Mutex::new(Vec::new()),
        });
        let opened_task = task_id.to_string();
        let mut module = RpcModule::new(shared.clone());
        module
            .register_method("message/send", move |params, shared, _| {
                let request: MessageSendRequest = params.one()?;
                shared
                    .received
                    .lock()
                    .expect("received")
                    .push(request.message);
                Ok::<MessageSendResponse, ErrorObjectOwned>(MessageSendResponse {
                    task_id: opened_task.clone(),
                    status: TaskStatus::Submitted,
                    response: None,
                })
            })
            .expect("message/send registers");
        module
            .register_method("tasks/get", |params, shared, _| {
                let _request: TaskGetRequest = params.one()?;
                match &shared.on_get {
                    OnGet::Task(task) => Ok(task.clone()),
                    OnGet::Error(code, message) => {
                        Err(ErrorObjectOwned::owned(*code, *message, None::<()>))
                    }
                }
            })
            .expect("tasks/get registers");

        let server = Server::builder()
            .build("127.0.0.1:0")
            .await
            .expect("loopback server");
        let address = format!("http://{}", server.local_addr().expect("bound address"));
        Self {
            address,
            shared,
            handle: server.start(module),
        }
    }

    fn received(&self) -> Vec<Message> {
        self.shared.received.lock().expect("received").clone()
    }

    async fn stop(self) {
        self.handle.stop().expect("server was running");
        self.handle.stopped().await;
    }
}

fn task(task_id: &str, status: TaskStatus) -> TaskGetResponse {
    TaskGetResponse {
        task_id: task_id.to_string(),
        status,
        result: None,
        error: None,
        progress: None,
    }
}

fn text(content: &str) -> MessagePart {
    MessagePart::Text {
        content: content.to_string(),
    }
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

async fn register_agent(state: &MeshToolsState, agent_id: &str, purpose: &str, caps: &[&str]) {
    state
        .registry
        .register_agent(
            agent_id.to_string(),
            agent_id.to_string(),
            purpose.to_string(),
            caps.iter().map(|c| (*c).to_string()).collect(),
            None,
            HashMap::new(),
            None,
        )
        .await
        .expect("agent registers");
}

#[spec("MESH-002")]
#[tokio::test]
async fn register_tools_registers_all_four_mesh_tools() {
    let state = Arc::new(MeshToolsState::new());
    let mut registry = ToolRegistry::empty();

    register_tools(&mut registry, state);

    for name in ["list_agents", "agent_query", "send_task", "get_task_status"] {
        let tool = registry
            .get(name)
            .unwrap_or_else(|| panic!("{name} is registered"));
        assert_eq!(tool.schema().name, name);
    }
}

#[spec("MESH-002")]
#[tokio::test]
async fn create_mesh_tools_returns_the_four_mesh_tools() {
    let state = Arc::new(MeshToolsState::new());

    let tools = create_mesh_tools(state);

    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool.schema().name.as_str())
        .collect();
    assert_eq!(
        names,
        ["list_agents", "agent_query", "send_task", "get_task_status"]
    );
}

#[spec("MESH-003")]
#[tokio::test]
async fn list_agents_returns_registered_agents_with_capabilities_and_addresses() {
    let state = Arc::new(MeshToolsState::new());
    register_agent(state.as_ref(), "reviewer", "reviews code", &["code_review"]).await;
    register_agent(
        state.as_ref(),
        "sentinel",
        "scans for vulnerabilities",
        &["security_analysis", "sast"],
    )
    .await;
    state
        .agent_addresses
        .write()
        .await
        .insert("sentinel".to_string(), "http://127.0.0.1:8200".to_string());

    let tool = ListAgentsTool::new(state);
    let result = tool.execute(json!({})).await.expect("listing succeeds");

    assert_eq!(result["success"], true);
    assert_eq!(result["agent_count"], 2);
    let agents = result["agents"].as_array().expect("agents is a list");
    let sentinel = agents
        .iter()
        .find(|a| a["agent_id"] == "sentinel")
        .expect("sentinel listed");
    assert_eq!(
        sentinel["capabilities"],
        json!(["security_analysis", "sast"])
    );
    assert_eq!(sentinel["purpose"], "scans for vulnerabilities");
    assert_eq!(sentinel["address"], "http://127.0.0.1:8200");
    assert_eq!(sentinel["is_available"], true);
    assert!(sentinel["last_seen"].as_str().is_some());
}

#[spec("MESH-003")]
#[tokio::test]
async fn list_agents_on_an_empty_mesh_returns_an_empty_list() {
    let state = Arc::new(MeshToolsState::new());
    let tool = ListAgentsTool::new(state);

    let result = tool.execute(json!({})).await.expect("listing succeeds");

    assert_eq!(result["success"], true);
    assert_eq!(result["agent_count"], 0);
    assert_eq!(result["agents"], json!([]));
}

#[spec("MESH-004")]
#[tokio::test]
async fn agent_query_filters_by_capability_and_prefers_less_busy_agents() {
    let state = Arc::new(MeshToolsState::new());
    register_agent(
        state.as_ref(),
        "busy-scan",
        "scans code",
        &["security_analysis"],
    )
    .await;
    register_agent(
        state.as_ref(),
        "idle-scan",
        "scans code",
        &["security_analysis"],
    )
    .await;
    register_agent(state.as_ref(), "reviewer", "reviews code", &["code_review"]).await;
    state
        .registry
        .update_agent_load("busy-scan", 0.8)
        .await
        .expect("load updates");
    state
        .registry
        .update_agent_load("idle-scan", 0.2)
        .await
        .expect("load updates");

    let tool = AgentQueryTool::new(state);
    let result = tool
        .execute(json!({"capability": "security_analysis"}))
        .await
        .expect("query succeeds");

    assert_eq!(result["success"], true);
    assert_eq!(result["match_count"], 2);
    let agents = result["agents"].as_array().expect("agents is a list");
    let ids: Vec<&str> = agents
        .iter()
        .map(|a| a["agent_id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, ["idle-scan", "busy-scan"], "least loaded first");
    assert_eq!(result["query"]["capability"], "security_analysis");
}

#[spec("MESH-004")]
#[tokio::test]
async fn agent_query_requires_a_filter_and_matches_purpose_text() {
    let state = Arc::new(MeshToolsState::new());
    register_agent(
        state.as_ref(),
        "sentinel",
        "Scans dependencies for known vulnerabilities",
        &["security_analysis"],
    )
    .await;
    register_agent(
        state.as_ref(),
        "reviewer",
        "Reviews pull requests",
        &["code_review"],
    )
    .await;
    let tool = AgentQueryTool::new(state);

    let rejected = tool.execute(json!({})).await.expect("query executes");
    assert_eq!(rejected["success"], false);
    assert!(
        rejected["error"]
            .as_str()
            .expect("error message")
            .contains("capability"),
        "{rejected}"
    );

    let result = tool
        .execute(json!({"purpose_contains": "VULNERABILITIES"}))
        .await
        .expect("query succeeds");
    assert_eq!(result["success"], true);
    assert_eq!(result["match_count"], 1);
    assert_eq!(result["agents"][0]["agent_id"], "sentinel");
}

#[spec("MESH-005")]
#[tokio::test]
async fn send_task_delivers_the_serialized_task_and_returns_its_id() {
    let agent =
        ScriptedAgent::start("task-42", OnGet::Task(task("task-42", TaskStatus::Working))).await;
    let state = Arc::new(MeshToolsState::new());
    state
        .agent_addresses
        .write()
        .await
        .insert("reviewer".to_string(), agent.address.clone());
    let tool = SendTaskTool::new(state.clone());

    let result = tool
        .execute(json!({"agent_id": "reviewer", "task": "review the diff"}))
        .await
        .expect("the peer accepts the task");

    assert_eq!(result["success"], true);
    assert_eq!(result["task_id"], "task-42");
    assert_eq!(result["agent_id"], "reviewer");
    assert_eq!(result["agent_address"], agent.address.as_str());

    let received = agent.received();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].parts.len(), 1);
    match &received[0].parts[0] {
        MessagePart::Text { content } => {
            assert_eq!(
                content, "review the diff",
                "the task text is serialized into the message"
            );
        }
        other => panic!("expected a text part, got {other:?}"),
    }
    let metadata = received[0].metadata.as_ref().expect("metadata added");
    assert!(
        metadata.get("budget_allocation").is_some(),
        "a budget allocation rides with the task: {metadata}"
    );

    let pending = state.pending_delegations.read().await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].task_id, "task-42");
    assert_eq!(pending[0].agent_id, "reviewer");
    agent.stop().await;
}

#[spec("MESH-005", "MESH-008")]
#[tokio::test]
async fn send_task_to_an_unreachable_agent_wraps_the_transport_error() {
    let state = Arc::new(MeshToolsState::new());
    state
        .agent_addresses
        .write()
        .await
        .insert("ghost".to_string(), vacant_address().await);
    let tool = SendTaskTool::new(state);

    let err = tool
        .execute(json!({"agent_id": "ghost", "task": "review the diff"}))
        .await
        .expect_err("nobody is listening");

    assert!(
        matches!(&err, MeshToolError::Execution(message) if message.contains("Failed to send task")),
        "transport error wrapped with context: {err}"
    );
}

#[spec("MESH-006")]
#[tokio::test]
async fn get_task_status_returns_the_current_status_and_result() {
    let mut finished = task("task-42", TaskStatus::Completed);
    finished.result = Some(Message {
        parts: vec![text("VERDICT: PASS")],
        metadata: None,
    });
    finished.progress = Some(TaskProgress {
        percentage: Some(100),
        message: Some("done".to_string()),
        eta_seconds: None,
    });
    let agent = ScriptedAgent::start("task-42", OnGet::Task(finished)).await;
    let state = Arc::new(MeshToolsState::new());
    state
        .agent_addresses
        .write()
        .await
        .insert("reviewer".to_string(), agent.address.clone());
    let tool = GetTaskStatusTool::new(state);

    let result = tool
        .execute(json!({"agent_id": "reviewer", "task_id": "task-42"}))
        .await
        .expect("status query succeeds");

    assert_eq!(result["success"], true);
    assert_eq!(result["task_id"], "task-42");
    assert_eq!(result["is_complete"], true);
    assert_eq!(result["result"], "VERDICT: PASS");
    assert_eq!(result["progress"]["percentage"], 100);
    agent.stop().await;
}

#[spec("MESH-006")]
#[tokio::test]
async fn get_task_status_reports_an_unknown_task_as_the_agents_answer() {
    let agent = ScriptedAgent::start("task-42", OnGet::Error(-32602, "Task not found")).await;
    let state = Arc::new(MeshToolsState::new());
    state
        .agent_addresses
        .write()
        .await
        .insert("reviewer".to_string(), agent.address.clone());
    let tool = GetTaskStatusTool::new(state);

    let result = tool
        .execute(json!({"agent_id": "reviewer", "task_id": "no-such-task"}))
        .await
        .expect("the agent answered");

    assert_eq!(result["success"], false);
    assert!(
        result["error"]
            .as_str()
            .expect("error message")
            .contains("Task not found"),
        "{result}"
    );
    agent.stop().await;
}

#[spec("MESH-006", "MESH-008")]
#[tokio::test]
async fn get_task_status_for_an_unknown_agent_names_the_agent() {
    let state = Arc::new(MeshToolsState::new());
    let tool = GetTaskStatusTool::new(state);

    let err = tool
        .execute(json!({"agent_id": "ghost", "task_id": "task-42"}))
        .await
        .expect_err("the agent is not known");

    assert!(
        matches!(&err, MeshToolError::Execution(message) if message.contains("Agent 'ghost' not found")),
        "context preserved: {err}"
    );
}

#[spec("MESH-008")]
#[tokio::test]
async fn missing_required_parameters_are_invalid_params_errors() {
    let state = Arc::new(MeshToolsState::new());

    let err = SendTaskTool::new(state.clone())
        .execute(json!({"agent_id": "reviewer"}))
        .await
        .expect_err("task is required");
    assert!(
        matches!(&err, MeshToolError::InvalidParams(message) if message == "task is required"),
        "{err}"
    );

    let err = GetTaskStatusTool::new(state)
        .execute(json!({"task_id": "task-42"}))
        .await
        .expect_err("agent_id is required");
    assert!(
        matches!(&err, MeshToolError::InvalidParams(message) if message == "agent_id is required"),
        "{err}"
    );
}

#[allow(dead_code)]
fn assert_value_debug(_: &Value) {}
