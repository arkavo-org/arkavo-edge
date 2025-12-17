//! API route definitions.

use std::sync::Arc;

use axum::{
    routing::{get, post, put},
    Router,
};
use tower_http::cors::{Any, CorsLayer};

use super::handlers;
use crate::config::Config;
use crate::profile::ProfileStore;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<ProfileStore>,
    pub config: Config,
}

/// Create the API router.
pub fn create_router(store: Arc<ProfileStore>, config: &Config) -> Router {
    let state = AppState {
        store,
        config: config.clone(),
    };

    // CORS configuration
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health check
        .route("/health", get(handlers::health_check))
        // Profile CRUD
        .route("/v1/profiles", post(handlers::publish_profile))
        .route("/v1/profiles/:public_id", get(handlers::get_profile))
        .route("/v1/profiles/:public_id", put(handlers::update_profile))
        // Discovery
        .route("/v1/profiles/search", get(handlers::search_profiles))
        .route("/v1/profiles/featured", get(handlers::get_featured_profiles))
        // Iroh ticket-based access
        .route(
            "/v1/profiles/ticket/:ticket",
            get(handlers::get_profile_by_ticket),
        )
        .layer(cors)
        .with_state(state)
}
