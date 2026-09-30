//! Discovery of an agent that listens on loopback, which is where an agent
//! started with `--bind 127.0.0.1` listens.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};

use super::{MeshToolsState, discover_and_register_agents};

/// Announces an agent bound to 127.0.0.1 on the loopback interfaces and on no
/// other, so the record can only reach a browser that listens there and
/// nothing the test publishes leaves the machine.
fn announce_on_loopback(agent_id: &str, port: u16) -> ServiceDaemon {
    let daemon = ServiceDaemon::new().expect("mDNS daemon");
    daemon
        .disable_interface(IfKind::All)
        .expect("leave every interface");
    daemon
        .enable_interface(vec![IfKind::LoopbackV4, IfKind::LoopbackV6])
        .expect("join the loopback interfaces");

    let mut properties = HashMap::new();
    properties.insert("agent_id".to_string(), agent_id.to_string());
    properties.insert("purpose".to_string(), "Plans the work".to_string());
    let service = ServiceInfo::new(
        "_a2a._tcp.local.",
        agent_id,
        &format!("{agent_id}.local."),
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        port,
        properties,
    )
    .expect("service info");
    daemon.register(service).expect("register the agent");
    daemon
}

/// Regression: the mesh tools browsed with a daemon that skips loopback
/// interfaces, so an agent on the same machine, listening on the default
/// address, was never found.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_listening_on_loopback_is_discovered() {
    let agent_id = format!("loopback-mesh-{}", std::process::id());
    let announcer = announce_on_loopback(&agent_id, 48_431);

    let state = MeshToolsState::new();
    discover_and_register_agents(&state)
        .await
        .expect("discovery runs");
    announcer.shutdown().ok();

    let addresses = state.agent_addresses.read().await;
    assert_eq!(
        addresses.get(&agent_id).map(String::as_str),
        Some("http://127.0.0.1:48431"),
        "discovered: {addresses:?}"
    );
}
