//! Shared fixtures for the gateway's unit tests.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::gateway::AppState;

/// Gateway state with nothing discovered, connected or stored.
pub(crate) fn app_state() -> AppState {
    AppState {
        connections: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        agents: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        agent_connections: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        budget_handler: Arc::new(tokio::sync::RwLock::new(
            crate::budget_handler::BudgetHandler::new(),
        )),
        cost_handler: Arc::new(tokio::sync::RwLock::new(
            crate::cost_handler::CostHandler::new(),
        )),
        security_handler: Arc::new(tokio::sync::RwLock::new(
            crate::security_handler::SecurityHandler::new(),
        )),
        initial_prompt: Arc::new(tokio::sync::RwLock::new(None)),
        dataflow_handler: Arc::new(crate::dataflow_handler::DataflowHandler::new()),
        debug_handler: None,
        rate_limiter: Arc::new(arkavo_protocol::rate_limit::IpRateLimiter::new(
            arkavo_protocol::rate_limit::RateLimitConfig::default(),
        )),
        task_store: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        learning_module: Arc::new(tokio::sync::RwLock::new(
            arkavo_router::learning::LearningModule::new(),
        )),
        routing_history: Arc::new(tokio::sync::RwLock::new(VecDeque::new())),
        lesson_tx: None,
        lesson_store: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        context_topology_cache: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        arp_handler: Arc::new(crate::arp_handler::ArpHandler::default()),
        swarm_flights: Arc::new(crate::swarm_flight_registry::SwarmFlightRegistry::new()),
        asset_fetcher: None,
    }
}

/// A task the workspace submitted, answered by the writer.
pub(crate) fn tracked_task(
    id: &str,
    status: &str,
    created_at: &str,
) -> crate::gateway::TrackedTask {
    crate::gateway::TrackedTask {
        id: id.into(),
        description: "Make a gold coin sprite".into(),
        target_agent: Some("writer".into()),
        status: status.into(),
        progress: Some(1.0),
        result: Some("Your gold coin sprite is attached.".into()),
        created_at: created_at.into(),
        completed_at: None,
        task_category: None,
        first_working_at: None,
        source: "ui".into(),
        summary: None,
        request_id: None,
        assets: Vec::new(),
        asset_note: None,
    }
}
