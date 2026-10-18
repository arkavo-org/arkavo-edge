//! Notices an agent reports reach the workspace once each, and keep
//! reaching it after the agent restarts (AGUI-027).
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;
use std::time::Duration;

use arkavo_agui::agent_connection::AgentConnection;
use arkavo_agui::security_handler::SecurityHandler;
use arkavo_agui::types::AgUiEvent;
use jsonrpsee::RpcModule;
use jsonrpsee::server::{Server, ServerHandle};
use serde_json::{Value, json};
use tokio::sync::{RwLock, mpsc};

fn notice(id: &str, revision: u32) -> Value {
    json!({
        "id": id, "key": "image-generation-failed", "agentId": "artist",
        "severity": "warning", "title": "Image generation failed",
        "text": "The GPU ran out of memory.", "count": revision, "revision": revision,
        "raisedAt": "2026-10-18T00:00:00Z",
        "actions": [{"id": "acknowledge", "label": "Acknowledge", "kind": "acknowledge"}]
    })
}

/// An agent on `port` whose metrics snapshots report `notices`, every 50 ms.
async fn agent(port: u16, notices: Value) -> (u16, ServerHandle) {
    let mut module = RpcModule::new(notices);
    module
        .register_subscription(
            "system.metrics.subscribe",
            "system.metrics.subscribe",
            "system.metrics.unsubscribe",
            |_, pending, notices, _| async move {
                let sink = pending.accept().await?;
                loop {
                    let snapshot = json!({"rss_mb": 1.0, "cpu_percent": 0.0, "pid": 1, "operator_notices": *notices});
                    if sink.send(serde_json::value::to_raw_value(&snapshot)?).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Ok::<(), jsonrpsee::core::SubscriptionError>(())
            },
        )
        .expect("registers");
    let server = Server::builder()
        .build(format!("127.0.0.1:{port}"))
        .await
        .expect("binds");
    let port = server.local_addr().expect("bound").port();
    (port, server.start(module))
}

async fn next_notice_event(rx: &mut mpsc::Receiver<AgUiEvent>) -> AgUiEvent {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let event = rx.recv().await.expect("relay running");
            if matches!(
                event,
                AgUiEvent::OperatorNotice { .. } | AgUiEvent::OperatorNoticeResolved { .. }
            ) {
                return event;
            }
        }
    })
    .await
    .expect("a notice event in time")
}

/// AGUI-027: a notice is relayed once although every snapshot repeats it;
/// a change is relayed, and one no longer reported is resolved. When the
/// agent restarts, the stream is opened again: before, it ended for good.
#[arkavo_test_macros::spec("AGUI-027")]
#[tokio::test]
async fn notices_are_relayed_once_and_survive_a_restart() {
    let (port, first) = agent(0, json!([notice("n1", 1)])).await;
    let (telemetry_tx, _telemetry) = mpsc::channel(64);
    let connection = Arc::new(AgentConnection::new(
        "artist".into(),
        format!("127.0.0.1:{port}"),
        telemetry_tx,
    ));
    connection.connect().await.expect("connects");
    let (tx, mut rx) = mpsc::channel(64);
    connection
        .subscribe_metrics(tx, Arc::new(RwLock::new(SecurityHandler::new())))
        .await
        .expect("subscribes");

    match next_notice_event(&mut rx).await {
        AgUiEvent::OperatorNotice { notice } => assert_eq!(notice.id, "n1"),
        other => panic!("expected the notice, got {other:?}"),
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event, AgUiEvent::OperatorNotice { .. }),
            "repeated: {event:?}"
        );
    }
    assert_eq!(connection.open_notices().await.len(), 1);

    first.stop().unwrap();
    first.stopped().await;
    let (_, second) = agent(port, json!([notice("n2", 1)])).await;

    let mut seen = Vec::new();
    for _ in 0..2 {
        seen.push(match next_notice_event(&mut rx).await {
            AgUiEvent::OperatorNotice { notice } => format!("open {}", notice.id),
            AgUiEvent::OperatorNoticeResolved { notice_id, .. } => format!("resolved {notice_id}"),
            _ => unreachable!(),
        });
    }
    seen.sort();
    assert_eq!(seen, ["open n2", "resolved n1"]);
    second.stop().unwrap();
}

/// A connection to the agent at `address`, relaying its snapshots, which
/// takes over the notices of `previous` first.
async fn subscribed(
    address: String,
    previous: Option<&AgentConnection>,
) -> (Arc<AgentConnection>, mpsc::Receiver<AgUiEvent>) {
    let (telemetry_tx, _telemetry) = mpsc::channel(64);
    let connection = Arc::new(AgentConnection::new("artist".into(), address, telemetry_tx));
    if let Some(previous) = previous {
        connection.inherit_notices(previous).await;
    }
    connection.connect().await.expect("connects");
    let (tx, rx) = mpsc::channel(64);
    connection
        .subscribe_metrics(tx, Arc::new(RwLock::new(SecurityHandler::new())))
        .await
        .expect("subscribes");
    (connection, rx)
}

/// Regression (AGUI-027): an agent found again on a new port got a fresh
/// connection with no notices, so views kept the old connection's notices
/// with nothing to close them, and answering one failed. The new
/// connection takes them over: the first snapshot closes the ones the
/// agent no longer reports and leaves the ones it still does alone.
#[arkavo_test_macros::spec("AGUI-027")]
#[tokio::test]
async fn an_agent_on_a_new_port_closes_only_what_it_no_longer_reports() {
    let (port, first) = agent(0, json!([notice("n1", 1), notice("kept", 1)])).await;
    let (old, mut old_rx) = subscribed(format!("127.0.0.1:{port}"), None).await;
    next_notice_event(&mut old_rx).await;
    next_notice_event(&mut old_rx).await;

    let (moved_port, second) = agent(0, json!([notice("kept", 1), notice("n2", 1)])).await;
    let (_moved, mut rx) = subscribed(format!("127.0.0.1:{moved_port}"), Some(&old)).await;

    let mut seen = Vec::new();
    for _ in 0..2 {
        seen.push(match next_notice_event(&mut rx).await {
            AgUiEvent::OperatorNotice { notice } => format!("open {}", notice.id),
            AgUiEvent::OperatorNoticeResolved { notice_id, .. } => format!("resolved {notice_id}"),
            _ => unreachable!(),
        });
    }
    seen.sort();
    assert_eq!(
        seen,
        ["open n2", "resolved n1"],
        "the kept notice is not shown again"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(
                event,
                AgUiEvent::OperatorNotice { .. } | AgUiEvent::OperatorNoticeResolved { .. }
            ),
            "unexpected: {event:?}"
        );
    }
    first.stop().unwrap();
    second.stop().unwrap();
}
