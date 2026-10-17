// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arkavo_protocol::types::AgentCapabilitiesGetResponse;
use arkavo_test_macros::spec;
use jsonrpsee::RpcModule;
use jsonrpsee::server::{Server, ServerHandle};
use serde_json::json;

use crate::MeshToolsState;

/// An agent that answers `agent.capabilities.get` and counts the calls.
async fn reporting_agent(
    image_generation: Option<serde_json::Value>,
) -> (String, Arc<AtomicUsize>, ServerHandle) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut module = RpcModule::new(calls.clone());
    module
        .register_method("agent.capabilities.get", move |_, calls, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            let mut report: AgentCapabilitiesGetResponse = serde_json::from_value(json!({
                "agent_id": "artist", "name": "artist", "purpose": "", "model": "gemma-4-12b",
                "capabilities": ["general"], "mcp_tools": [], "load": 0.0,
                "accepting_tasks": true, "public_key": "", "version": "1.0"
            }))
            .expect("a report");
            report.image_generation.clone_from(&image_generation);
            Ok::<_, jsonrpsee::types::ErrorObjectOwned>(report)
        })
        .expect("registers");
    let server = Server::builder()
        .build("127.0.0.1:0")
        .await
        .expect("loopback server");
    let address = format!("http://{}", server.local_addr().expect("bound"));
    (address, calls, server.start(module))
}

async fn vacant_address() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    address
}

/// MESH-010: capabilities come from the member's own report, read once per
/// minute; a member that does not answer is left out.
#[spec("MESH-010")]
#[tokio::test]
async fn reports_are_read_from_members_and_cached() {
    let descriptor = json!({"chip": "Apple M6", "models": []});
    let (address, calls, handle) = reporting_agent(Some(descriptor.clone())).await;
    let mesh = MeshToolsState::new();
    let gone = vacant_address().await;
    mesh.agent_addresses
        .write()
        .await
        .extend([("artist".into(), address), ("gone".into(), gone)]);

    let reports = mesh.member_reports().await;
    assert_eq!(reports.members.len(), 1, "{reports:?}");
    assert_eq!(reports.members[0].agent_id, "artist");
    assert_eq!(reports.members[0].report.image_generation, Some(descriptor));
    assert_eq!(reports.silent, ["gone"]);
    assert_eq!(mesh.member_reports().await.members.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the second read is cached");

    handle.stop().unwrap();
    handle.stopped().await;
}

/// A member that moved is read again at its new address.
#[tokio::test]
async fn a_new_address_is_read_again() {
    let (first, first_calls, first_handle) = reporting_agent(None).await;
    let (second, second_calls, second_handle) = reporting_agent(None).await;
    let mesh = MeshToolsState::new();
    mesh.agent_addresses
        .write()
        .await
        .insert("artist".into(), first);
    assert!(
        mesh.member_reports().await.members[0]
            .report
            .image_generation
            .is_none()
    );
    mesh.agent_addresses
        .write()
        .await
        .insert("artist".into(), second);
    assert_eq!(mesh.member_reports().await.members.len(), 1);
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 1);
    for handle in [first_handle, second_handle] {
        handle.stop().unwrap();
        handle.stopped().await;
    }
}
