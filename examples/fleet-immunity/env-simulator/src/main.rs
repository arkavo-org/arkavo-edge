//! Fleet Immunity Environment Simulator
//!
//! Simulates a warehouse environment with sectors and hazards.
//! Provides HTTP API for hazard injection and sector queries.

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Clone)]
struct AppState {
    sectors: Arc<RwLock<HashMap<u8, Sector>>>,
}

#[derive(Clone, Serialize)]
struct Sector {
    id: u8,
    name: String,
    hazard: Option<Hazard>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Hazard {
    #[serde(rename = "type")]
    hazard_type: String,
    traction: f32,
}

#[derive(Deserialize)]
struct InjectRequest {
    sector: u8,
    hazard: String,
    traction: f32,
}

#[derive(Serialize)]
struct InjectResponse {
    success: bool,
    message: String,
}

#[derive(Serialize)]
struct StatusResponse {
    warehouse: String,
    sectors: Vec<Sector>,
}

#[tokio::main]
async fn main() {
    let mut sectors = HashMap::new();
    sectors.insert(1, Sector { id: 1, name: "Loading Bay".into(), hazard: None });
    sectors.insert(2, Sector { id: 2, name: "Storage Aisle A".into(), hazard: None });
    sectors.insert(3, Sector { id: 3, name: "Storage Aisle B".into(), hazard: None });
    sectors.insert(4, Sector { id: 4, name: "Cold Storage".into(), hazard: None });

    let state = AppState {
        sectors: Arc::new(RwLock::new(sectors)),
    };

    let app = Router::new()
        .route("/status", get(status))
        .route("/inject", post(inject))
        .route("/sector/:id", get(get_sector))
        .with_state(state);

    println!("[ENV] Warehouse environment simulator started on port 8360");
    println!("[ENV] Sectors: Loading Bay, Storage Aisle A, Storage Aisle B, Cold Storage");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8360").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let sectors = state.sectors.read().unwrap();
    let mut sector_list: Vec<Sector> = sectors.values().cloned().collect();
    sector_list.sort_by_key(|s| s.id);

    Json(StatusResponse {
        warehouse: "Demo Warehouse".into(),
        sectors: sector_list,
    })
}

async fn inject(
    State(state): State<AppState>,
    Json(req): Json<InjectRequest>,
) -> Json<InjectResponse> {
    let mut sectors = state.sectors.write().unwrap();

    if let Some(sector) = sectors.get_mut(&req.sector) {
        sector.hazard = Some(Hazard {
            hazard_type: req.hazard.clone(),
            traction: req.traction,
        });

        println!("[ENV] HAZARD INJECTED: {} in Sector {} ({})", req.hazard, req.sector, sector.name);
        println!("[ENV] Traction coefficient: {}", req.traction);

        Json(InjectResponse {
            success: true,
            message: format!("Hazard '{}' injected into sector {}", req.hazard, req.sector),
        })
    } else {
        Json(InjectResponse {
            success: false,
            message: format!("Sector {} not found", req.sector),
        })
    }
}

async fn get_sector(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<u8>,
) -> Json<Option<Sector>> {
    let sectors = state.sectors.read().unwrap();
    Json(sectors.get(&id).cloned())
}
