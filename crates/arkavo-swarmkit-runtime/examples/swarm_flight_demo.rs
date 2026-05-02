//! End-to-end SwarmFlight demo. Loads a SwarmKit manifest, launches a flight,
//! drives synthetic per-role tool outcomes, and prints each role's
//! DecisionTrace summary.
//!
//! Usage:
//!   cargo run -p arkavo-swarmkit-runtime --example swarm_flight_demo \
//!     -- examples/campaign-kit/campaign-kit.swarmkit.yaml

use std::path::PathBuf;

use arkavo_swarmkit::{parse_json, parse_yaml};
use arkavo_swarmkit_runtime::{LaunchOptions, SwarmFlight};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: swarm_flight_demo <path-to-manifest>");
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

    let flight = SwarmFlight::launch(&manifest, LaunchOptions::default()).expect("launch");
    println!("== SwarmFlight launched ==");
    println!("kit:        {}", flight.kit_name());
    println!("kit_id:     {}", flight.kit_id());
    println!("flight_id:  {}", flight.flight_id());
    println!();

    // Synthetic pipeline: drive at least one outcome per role with realistic
    // quality scores. Treats role_type as a hint to choose a tool name.
    for role in flight.roles() {
        let (tool_name, success, quality) = synthesize_outcome(role.role_type());
        flight
            .record_tool_outcome(role.role_id(), tool_name, success, quality)
            .await
            .expect("record");
    }

    println!("== Per-role DecisionTrace summary ==");
    for role in flight.roles() {
        let trace = role.arp().decision_trace();
        let cache = role.arp().cache();
        let snap = trace.snapshot();
        println!(
            "[{}] {} ({}) — {} trace entr{}, {} cache entr{}, gate={:.2}",
            role.role_id(),
            role.role_type(),
            if snap.is_empty() {
                "no events"
            } else {
                "active"
            },
            snap.len(),
            if snap.len() == 1 { "y" } else { "ies" },
            cache.len(),
            if cache.len() == 1 { "y" } else { "ies" },
            role.arp().quality_threshold(),
        );
        for entry in &snap {
            println!(
                "    - tool={:?} success={:?} quality={:?} error_type={:?}",
                entry.decision.chosen,
                entry.outcome.success,
                entry.outcome.quality_score,
                entry.outcome.error_type,
            );
        }
    }

    println!();
    println!(
        "Cache chain valid for all roles: {}",
        flight.roles().all(|r| r.arp().cache().verify_chain())
    );
}

fn synthesize_outcome(role_type: &str) -> (&'static str, bool, f64) {
    match role_type {
        "asset_analyst" => ("asset.summarize", true, 0.91),
        "platform_copy" => ("copy.draft_post", true, 0.83),
        "critic" => ("critic.score_rubric", true, 0.78),
        "scribe" => ("transcribe.event", true, 0.95),
        "historian" => ("context.aggregate", true, 0.88),
        "planner" => ("plan.decompose", true, 0.82),
        "operator" => ("action.submit", true, 0.85),
        _ => ("generic.tool", true, 0.80),
    }
}
