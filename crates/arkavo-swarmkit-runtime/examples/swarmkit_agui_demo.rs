//! End-to-end demo: SwarmKit spec → SwarmFlight runtime → AG-UI ARP panel.
//!
//! Loads a SwarmKit manifest, launches a flight, registers each role with an
//! `ArpHandler` (the same handler the AG-UI gateway uses), drives synthetic
//! tool outcomes, and prints the JSON snapshot the WebSocket panel would
//! consume — proving the end-to-end loop works on a real manifest.
//!
//! Usage:
//!   cargo run -p arkavo-swarmkit-runtime --example swarmkit_agui_demo \
//!     -- examples/campaign-kit/campaign-kit.swarmkit.yaml

use std::path::PathBuf;
use std::sync::Arc;

use arkavo_agui::arp_handler::{ArpHandler, FlightRoleRegistration};
use arkavo_swarmkit::{parse_json, parse_yaml};
use arkavo_swarmkit_runtime::{LaunchOptions, SwarmFlight};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: swarmkit_agui_demo <path-to-manifest>");
        std::process::exit(2);
    };
    let path = PathBuf::from(path);
    let raw = std::fs::read_to_string(&path).expect("read manifest");
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let manifest = if is_json {
        parse_json(&raw).expect("parse JSON manifest")
    } else {
        parse_yaml(&raw).expect("parse YAML manifest")
    };

    // 1. Launch a SwarmFlight — builds one ArpRuntime per role.
    let flight = SwarmFlight::launch(&manifest, LaunchOptions::default()).expect("launch");
    println!("== SwarmFlight launched ==");
    println!("kit:       {}", flight.kit_name());
    println!("flight_id: {}", flight.flight_id());
    println!("roles:     {}", flight.roles().count());
    println!();

    // 2. Stand up an ArpHandler — same struct the AG-UI gateway instantiates.
    let handler = Arc::new(ArpHandler::new());

    // 3. Register every role of the flight. Each becomes addressable in the
    //    panel's per-agent dropdown, tagged with FlightContext so the UI can
    //    render kit + role metadata instead of the synthetic agent_id.
    for role in flight.roles() {
        handler
            .attach_flight_role(FlightRoleRegistration {
                flight_id: flight.flight_id(),
                kit_id: flight.kit_id().to_string(),
                kit_name: flight.kit_name().to_string(),
                role_id: role.role_id().to_string(),
                role_type: role.role_type().to_string(),
                arp_doc: role.arp_document().clone(),
                runtime: role.arp().clone(),
            })
            .await;
    }

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
