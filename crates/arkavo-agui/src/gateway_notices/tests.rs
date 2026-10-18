#![allow(clippy::disallowed_methods)]

use arkavo_test_macros::spec;
use tokio::sync::mpsc;

use super::*;

fn reported(id: &str, revision: u32) -> OperatorNotice {
    OperatorNotice {
        id: id.into(),
        key: "image-generation-failed".into(),
        agent_id: "artist".into(),
        severity: NoticeSeverity::Warning,
        title: "Image generation failed".into(),
        text: "x".into(),
        focus: None,
        details: Vec::new(),
        actions: Vec::new(),
        count: revision,
        revision,
        raised_at: "2026-10-18T00:00:00Z".into(),
    }
}

fn kinds(events: &[AgUiEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| match e {
            AgUiEvent::OperatorNotice { notice } => {
                format!("open {} r{}", notice.id, notice.revision)
            }
            AgUiEvent::OperatorNoticeResolved { notice_id, .. } => format!("resolved {notice_id}"),
            other => format!("{other:?}"),
        })
        .collect()
}

/// AGUI-027: snapshots repeat what is open; views hear each change once.
#[spec("AGUI-027")]
#[test]
fn only_changes_are_relayed() {
    let mut open = Vec::new();
    assert_eq!(
        kinds(&changes(&mut open, vec![reported("n1", 1)])),
        ["open n1 r1"]
    );
    assert!(
        changes(&mut open, vec![reported("n1", 1)]).is_empty(),
        "a repeat is not news"
    );
    assert_eq!(
        kinds(&changes(&mut open, vec![reported("n1", 2)])),
        ["open n1 r2"]
    );
    assert_eq!(kinds(&changes(&mut open, Vec::new())), ["resolved n1"]);
    assert!(open.is_empty());
}

async fn view(
    connections: &Arc<RwLock<HashMap<String, ConnectionInfo>>>,
) -> mpsc::Receiver<AgUiEvent> {
    let (tx, rx) = mpsc::channel(8);
    connections.write().await.insert(
        "view".into(),
        ConnectionInfo {
            _ws_tx: tx,
            _agent_id: None,
            subscriptions: Vec::new(),
            current_plan: None,
            current_prompt: None,
        },
    );
    rx
}

/// AGUI-028: a task the workspace submitted that fails is brought to every
/// view, with the task to open and a way to submit it again.
#[spec("AGUI-028")]
#[tokio::test]
async fn a_failed_task_is_a_notice_with_retry() {
    let state = crate::test_support::app_state();
    let mut rx = view(&state.connections).await;

    task_failed(
        &state.gateway_notices,
        &state.connections,
        "t1",
        "writer",
        "The GPU ran out of memory.",
    )
    .await;

    let AgUiEvent::OperatorNotice { notice } = rx.recv().await.unwrap() else {
        panic!("expected a notice");
    };
    assert_eq!(
        notice.focus.as_ref().and_then(|f| f.task.as_deref()),
        Some("t1")
    );
    let actions: Vec<_> = notice.actions.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(actions, ["inspect", "retry", "acknowledge"]);
    assert_eq!(
        all_open(&state.gateway_notices, &state.agent_connections)
            .await
            .len(),
        1
    );
}

/// AGUI-028: retrying closes the notice and gives back the task's
/// objective to submit again; dismissing closes it; an action it does not
/// offer is refused.
#[spec("AGUI-028")]
#[tokio::test]
async fn gateway_notice_actions_are_carried_out() {
    let state = crate::test_support::app_state();
    state.task_store.write().await.insert(
        "t1".into(),
        crate::test_support::tracked_task("t1", "failed", "2026-10-18T00:00:00Z"),
    );
    let mut rx = view(&state.connections).await;
    task_failed(
        &state.gateway_notices,
        &state.connections,
        "t1",
        "writer",
        "x",
    )
    .await;
    let AgUiEvent::OperatorNotice { notice } = rx.recv().await.unwrap() else {
        panic!("expected a notice");
    };
    let act_on = |action: &'static str| {
        act(
            &state.gateway_notices,
            &state.agent_connections,
            &state.task_store,
            &state.connections,
            "writer",
            &notice.id,
            action,
        )
    };

    assert!(
        act_on("pause")
            .await
            .unwrap_err()
            .contains("no action pause")
    );
    assert!(matches!(act_on("inspect").await, Ok(Acted::Opened)));
    match act_on("retry").await {
        Ok(Acted::Retry(objective)) => assert_eq!(objective, "Make a gold coin sprite"),
        _ => panic!("expected a retry"),
    }
    assert!(matches!(
        rx.recv().await.unwrap(),
        AgUiEvent::OperatorNoticeResolved { .. }
    ));
    assert!(
        all_open(&state.gateway_notices, &state.agent_connections)
            .await
            .is_empty()
    );
}

/// An agent whose snapshots report one notice and which records the
/// answers it receives.
async fn answering_agent() -> (
    String,
    Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    jsonrpsee::server::ServerHandle,
) {
    let answers = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut module = jsonrpsee::RpcModule::new(answers.clone());
    module
        .register_subscription(
            "system.metrics.subscribe",
            "system.metrics.subscribe",
            "system.metrics.unsubscribe",
            |_, pending, _, _| async move {
                let sink = pending.accept().await?;
                let mut notice = serde_json::to_value(reported("n1", 1)).unwrap();
                notice["actions"] = serde_json::json!([{"id": "acknowledge", "label": "Acknowledge", "kind": "acknowledge"}]);
                let snapshot = serde_json::json!({"rss_mb": 1.0, "cpu_percent": 0.0, "pid": 1, "operator_notices": [notice]});
                while sink.send(serde_json::value::to_raw_value(&snapshot)?).await.is_ok() {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Ok::<(), jsonrpsee::core::SubscriptionError>(())
            },
        )
        .unwrap();
    module
        .register_method("operator.notices.respond", |params, answers, _| {
            let response: serde_json::Value = params.one()?;
            answers.lock().unwrap().push(response);
            Ok::<_, jsonrpsee::types::ErrorObjectOwned>(serde_json::json!({"accepted": true}))
        })
        .unwrap();
    let server = jsonrpsee::server::Server::builder()
        .build("127.0.0.1:0")
        .await
        .unwrap();
    let address = server.local_addr().unwrap().to_string();
    (address, answers, server.start(module))
}

/// AGUI-029: acknowledging an agent's notice reaches that agent, and once
/// it accepts, every view hears the notice closed.
#[spec("AGUI-029")]
#[tokio::test]
async fn acknowledging_an_agents_notice_reaches_the_agent() {
    let (address, answers, server) = answering_agent().await;
    let state = crate::test_support::app_state();
    let (telemetry_tx, _telemetry) = mpsc::channel(64);
    let connection = Arc::new(AgentConnection::new("artist".into(), address, telemetry_tx));
    connection.connect().await.unwrap();
    let (relay_tx, mut relay_rx) = mpsc::channel(64);
    connection
        .subscribe_metrics(relay_tx, state.security_handler.clone())
        .await
        .unwrap();
    while !matches!(
        relay_rx.recv().await,
        Some(AgUiEvent::OperatorNotice { .. })
    ) {}
    state
        .agent_connections
        .write()
        .await
        .insert("artist".into(), connection.clone());
    let mut rx = view(&state.connections).await;

    let acted = act(
        &state.gateway_notices,
        &state.agent_connections,
        &state.task_store,
        &state.connections,
        "artist",
        "n1",
        "acknowledge",
    )
    .await;

    assert!(matches!(acted, Ok(Acted::Closed)), "{acted:?}");
    assert_eq!(
        *answers.lock().unwrap(),
        [serde_json::json!({"noticeId": "n1", "actionId": "acknowledge"})]
    );
    assert!(
        matches!(rx.recv().await.unwrap(), AgUiEvent::OperatorNoticeResolved { notice_id, .. } if notice_id == "n1")
    );
    server.stop().unwrap();
}

/// Regression (AGUI-027): when the gateway dropped an agent whose
/// discovery record went away, views kept that agent's notices with
/// nothing left to close them.
#[spec("AGUI-027")]
#[tokio::test]
async fn a_dropped_agents_notices_close_in_every_view() {
    let (address, _answers, server) = answering_agent().await;
    let state = crate::test_support::app_state();
    let (telemetry_tx, _telemetry) = mpsc::channel(64);
    let connection = Arc::new(AgentConnection::new("artist".into(), address, telemetry_tx));
    connection.connect().await.unwrap();
    let (relay_tx, mut relay_rx) = mpsc::channel(64);
    connection
        .subscribe_metrics(relay_tx, state.security_handler.clone())
        .await
        .unwrap();
    while !matches!(
        relay_rx.recv().await,
        Some(AgUiEvent::OperatorNotice { .. })
    ) {}
    let mut rx = view(&state.connections).await;

    connection_removed(&connection, &state.connections).await;

    assert!(
        matches!(rx.recv().await.unwrap(), AgUiEvent::OperatorNoticeResolved { notice_id, .. } if notice_id == "n1")
    );
    server.stop().unwrap();
}
