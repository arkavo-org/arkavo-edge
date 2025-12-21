//! Rover Simulator for Fleet Immunity Demo
//!
//! Simulates an autonomous rover navigating through warehouse sectors,
//! detecting hazards, crashing, learning, and sharing lessons with the fleet.

use axum::{extract::State, routing::post, Json, Router};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Parser)]
#[command(name = "rover-simulator")]
struct Args {
    /// Rover name (e.g., "alpha", "beta", "gamma")
    #[arg(short, long)]
    name: String,

    /// Port to listen on for lesson broadcasts
    #[arg(short, long)]
    port: u16,

    /// Route as comma-separated sector IDs (e.g., "1,2,4,3")
    #[arg(short, long)]
    route: String,

    /// Peer rover URLs (comma-separated)
    #[arg(long, default_value = "")]
    peers: String,

    /// Environment simulator URL
    #[arg(long, default_value = "http://localhost:8360")]
    env_url: String,

    /// Navigation speed in ms between sectors
    #[arg(long, default_value = "2000")]
    speed_ms: u64,
}

#[derive(Clone)]
struct RoverState {
    name: String,
    policies: Arc<RwLock<HashSet<u8>>>, // Sectors to slow down in
    peers: Vec<String>,
    env_url: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Lesson {
    from_rover: String,
    sector: u8,
    rule: String,
    reason: String,
}

#[derive(Deserialize)]
struct Sector {
    id: u8,
    name: String,
    hazard: Option<Hazard>,
}

#[derive(Deserialize)]
struct Hazard {
    #[serde(rename = "type")]
    hazard_type: String,
    traction: f32,
}

fn log(name: &str, msg: &str) {
    let prefix = match name.to_uppercase().as_str() {
        "ALPHA" => "[ALPHA ]",
        "BETA" => "[BETA  ]",
        "GAMMA" => "[GAMMA ]",
        _ => "[ROVER ]",
    };
    println!("{} {}", prefix, msg);
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let route: Vec<u8> = args
        .route
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    let peers: Vec<String> = args
        .peers
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let state = RoverState {
        name: args.name.clone(),
        policies: Arc::new(RwLock::new(HashSet::new())),
        peers,
        env_url: args.env_url,
    };

    // Start HTTP server for receiving lessons
    let server_state = state.clone();
    let port = args.port;
    tokio::spawn(async move {
        let app = Router::new()
            .route("/lesson", post(receive_lesson))
            .with_state(server_state);

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
            .await
            .unwrap();
        axum::serve(listener, app).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    log(&args.name, &format!("Rover initialized - Route: {:?}", route));
    log(&args.name, &format!("Listening on port {}", port));

    // Navigation loop
    let client = reqwest::Client::new();
    let speed_ms = args.speed_ms;

    for sector_id in route.iter().cycle() {
        tokio::time::sleep(tokio::time::Duration::from_millis(speed_ms)).await;

        // Check if we have a policy for this sector
        let should_slow = state.policies.read().await.contains(sector_id);
        let speed = if should_slow { "SLOW" } else { "FAST" };

        log(&args.name, &format!("Entering Sector {}...", sector_id));
        log(&args.name, &format!("Speed: >>> {} >>>", speed));

        // Query environment for sector status
        let sector_url = format!("{}/sector/{}", state.env_url, sector_id);
        if let Ok(resp) = client.get(&sector_url).send().await {
            if let Ok(Some(sector)) = resp.json::<Option<Sector>>().await {
                if let Some(hazard) = sector.hazard {
                    log(&args.name, "TRACTION LOSS DETECTED!");

                    if speed == "FAST" {
                        // CRASH!
                        println!();
                        log(&args.name, "████████████████████████████████████████");
                        log(&args.name, "███           >>> CRASH <<<          ███");
                        log(&args.name, "████████████████████████████████████████");
                        println!();
                        log(
                            &args.name,
                            &format!(
                                "[PAIN] Driving fast while sliding! ({})",
                                hazard.hazard_type
                            ),
                        );

                        // Synthesize lesson
                        println!();
                        log(&args.name, "Synthesizing safety lesson...");
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                        let lesson = Lesson {
                            from_rover: args.name.clone(),
                            sector: *sector_id,
                            rule: format!("IF Sector_{} THEN Drive_Slow", sector_id),
                            reason: format!(
                                "{} in {} (traction: {})",
                                hazard.hazard_type, sector.name, hazard.traction
                            ),
                        };

                        log(&args.name, &format!("[OK] Lesson: \"{}\"", lesson.rule));

                        // Apply to self
                        state.policies.write().await.insert(*sector_id);

                        // Broadcast to fleet
                        log(&args.name, "Broadcasting to fleet...");
                        broadcast_lesson(&state, &lesson).await;

                        println!();
                    } else {
                        // Slowed down, avoided crash!
                        println!();
                        log(&args.name, "════════════════════════════════════════");
                        log(&args.name, "═══      >>> SLOWING DOWN <<<       ═══");
                        log(&args.name, "════════════════════════════════════════");
                        println!();
                        log(&args.name, "[OK] CRASH AVOIDED - Learned from fleet!");
                        println!();
                    }
                } else {
                    log(&args.name, &format!("Sector {} >>> [OK]", sector_id));
                }
            }
        }
    }
}

async fn broadcast_lesson(state: &RoverState, lesson: &Lesson) {
    let client = reqwest::Client::new();

    for peer in &state.peers {
        let url = format!("{}/lesson", peer);
        match client.post(&url).json(lesson).send().await {
            Ok(_) => log(&state.name, &format!("Sent lesson to {}", peer)),
            Err(e) => log(&state.name, &format!("Failed to send to {}: {}", peer, e)),
        }
    }
}

async fn receive_lesson(
    State(state): State<RoverState>,
    Json(lesson): Json<Lesson>,
) -> Json<serde_json::Value> {
    log(
        &state.name,
        &format!("Received lesson from {} via A2A", lesson.from_rover.to_uppercase()),
    );
    log(&state.name, &format!("  Rule: \"{}\"", lesson.rule));
    log(&state.name, &format!("  Reason: {}", lesson.reason));

    // Verify independently (simulated)
    log(&state.name, "Verifying independently...");
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Apply the policy
    state.policies.write().await.insert(lesson.sector);
    log(&state.name, "Voting: APPROVE");

    Json(serde_json::json!({"status": "approved"}))
}
