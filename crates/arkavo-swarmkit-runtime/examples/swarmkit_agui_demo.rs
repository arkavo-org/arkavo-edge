//! End-to-end demo: SwarmKit spec → SwarmFlight runtime → AG-UI ARP panel.
//!
//! Mirrors what the gateway does at boot when `ARKAVO_SWARMKIT_PATH` is set:
//! parse the manifest, launch a flight, register it with the gateway's
//! `SwarmFlightRegistry` (which attaches every role to the `ArpHandler`),
//! drive a few tool outcomes, and print the JSON the WebSocket panel
//! consumes.
//!
//! Usage:
//!   cargo run -p arkavo-swarmkit-runtime --example swarmkit_agui_demo \
//!     -- examples/campaign-kit/campaign-kit.swarmkit.yaml

use std::sync::Arc;

use arkavo_agui::arp_handler::ArpHandler;
use arkavo_agui::swarm_flight_registry::{SwarmFlightRegistry, launch_from_path};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: swarmkit_agui_demo <path-to-manifest>");
        std::process::exit(2);
    };

    // 1. Launch a SwarmFlight — same code path the gateway uses for its
    //    ARKAVO_SWARMKIT_PATH auto-launch.
    let flight = Arc::new(launch_from_path(std::path::Path::new(&path)).expect("launch"));
    println!("== SwarmFlight launched ==");
    println!("kit:       {}", flight.kit_name());
    println!("flight_id: {}", flight.flight_id());
    println!("roles:     {}", flight.roles().count());
    println!();

    // 2. Stand up the same gateway components: ArpHandler + SwarmFlightRegistry.
    let handler = Arc::new(ArpHandler::new());
    let registry = SwarmFlightRegistry::new();

    // 3. Register the flight. The registry handles per-role attachment to
    //    the ArpHandler in the order the manifest declared.
    registry.register(flight.clone(), &handler).await;

    // 4. Drive synthetic outcomes per role.
    for role in flight.roles() {
        let (tool, success, quality) = match role.role_type() {
            "asset_analyst" => ("asset.summarize", true, 0.91),
            "platform_copy" => ("copy.draft_post", true, 0.83),
            "critic" => ("critic.score_rubric", true, 0.78),
            _ => ("generic.tool", true, 0.80),
        };
        flight
            .record_tool_outcome(role.role_id(), tool, success, quality)
            .await
            .unwrap();
    }

    // 5. Snapshot the handler — same payload the WebSocket panel consumes.
    let snapshot = handler.snapshot().await;
    let json = serde_json::to_string_pretty(&snapshot).unwrap();

    println!("== AG-UI ArpStatusSnapshot (WebSocket payload) ==");
    println!("{json}");

    // 6. Cross-check: every flight role surfaces with FlightContext.
    let flight_count = snapshot
        .agents
        .iter()
        .filter(|a| a.flight_context.is_some())
        .count();
    println!();
    println!(
        "Snapshot has {} agent(s); {} carry flightContext (= {} flight roles).",
        snapshot.agents.len(),
        flight_count,
        flight.roles().count()
    );
    assert_eq!(
        flight_count,
        flight.roles().count(),
        "every role must surface with flight context"
    );
    println!("End-to-end loop verified.");
}
