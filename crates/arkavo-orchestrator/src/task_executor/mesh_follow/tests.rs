//! The follow loop against an agent on loopback that scripts its answers to
//! `tasks/get`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use arkavo_protocol::http::HttpTransport;
use arkavo_protocol::rate_limit::{RATE_LIMITED, RATE_LIMITED_MESSAGE};
use arkavo_protocol::transport::{A2aEndpoint, TlsConfig, TransportConfig};
use arkavo_test_macros::spec;
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};

use super::*;
use crate::task_executor::ui::{MessageType, MockUI};

/// An agent whose answers to `tasks/get` are `script`, in order, the last
/// repeated once the script runs out. Each is a JSON-RPC result or error.
async fn agent(script: Vec<Value>) -> HttpTransport {
    let polls = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        "/rpc",
        post(move |Json(request): Json<Value>| {
            let polls = polls.clone();
            let script = script.clone();
            async move {
                let n = polls.fetch_add(1, Ordering::SeqCst).min(script.len() - 1);
                let mut answer = script[n].clone();
                answer["jsonrpc"] = json!("2.0");
                answer["id"] = request["id"].clone();
                Json(answer)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback");
    let url = format!("http://{}", listener.local_addr().expect("bound"));
    tokio::spawn(async move { axum::serve(listener, app).await });
    let transport = HttpTransport::new(TransportConfig {
        tls_config: TlsConfig {
            require_tls: false,
            ..Default::default()
        },
        ..Default::default()
    })
    .expect("transport");
    transport
        .connect(&A2aEndpoint {
            url,
            agent_id: "writer".into(),
            public_key: None,
        })
        .await
        .expect("connects");
    transport
}

fn refused() -> Value {
    json!({"error": {"code": RATE_LIMITED, "message": RATE_LIMITED_MESSAGE}})
}

fn working(step: u32) -> Value {
    json!({"result": {
        "task_id": "t1",
        "status": "working",
        "progress": {"message": format!("image 1 of 1: step {step} of 8")}
    }})
}

fn completed() -> Value {
    json!({"result": {
        "task_id": "t1",
        "status": "completed",
        "result": {"parts": [
            {"type": "text", "content": "Your gold coin sprite is attached."},
            {"type": "data", "schema": "urn:arkavo:asset:manifest:v1", "content": {}}
        ]}
    }})
}

/// Regression (CLI-018): `arkavo task` reported "Completed via mesh agent"
/// and dropped the agent's answer, so the text and the assets it delivered
/// never reached the caller. The result is the answer.
#[spec("CLI-018")]
#[tokio::test]
async fn a_completed_task_is_the_agents_answer() {
    let transport = agent(vec![working(1), completed()]).await;
    let ui = MockUI::new();

    let result = follow(&transport, "t1", &ui, Watch::default())
        .await
        .expect("followed");

    assert!(result.success);
    assert_eq!(result.message, "Your gold coin sprite is attached.");
    let answer = result.answer.expect("the answer is kept");
    assert_eq!(answer.task_id, "t1");
    assert_eq!(answer.message.parts.len(), 2, "the asset part is kept");
}

/// Regression (CLI-019): `arkavo task` gave up five minutes after it
/// submitted, whatever the task reported, and a refused poll ended it. A
/// refused poll is made again, and each new progress message is shown once.
#[spec("CLI-019")]
#[tokio::test]
async fn refused_polls_are_made_again_and_progress_is_shown_once() {
    let transport = agent(vec![
        refused(),
        working(1),
        working(1),
        refused(),
        working(2),
        completed(),
    ])
    .await;
    let ui = MockUI::new();

    let result = follow(&transport, "t1", &ui, Watch::default())
        .await
        .expect("followed through the refusals");

    assert!(result.success);
    let progress: Vec<_> = ui
        .messages()
        .into_iter()
        .filter(|(kind, _)| *kind == MessageType::Progress)
        .map(|(_, message)| message)
        .collect();
    assert_eq!(
        progress,
        ["image 1 of 1: step 1 of 8", "image 1 of 1: step 2 of 8"]
    );
}

/// CLI-019: a task that reports nothing new for the limit is left running,
/// and the error says how to read its result later.
#[spec("CLI-019")]
#[tokio::test]
async fn a_quiet_task_is_left_running() {
    let transport = agent(vec![working(3)]).await;

    let error = follow(
        &transport,
        "t1",
        &MockUI::new(),
        Watch::with_limit(Duration::ZERO),
    )
    .await
    .expect_err("quiet past the limit");

    let error = error.to_string();
    assert!(error.contains("Task t1 is still running"), "{error}");
    assert!(error.contains("tasks/get"), "{error}");
}
