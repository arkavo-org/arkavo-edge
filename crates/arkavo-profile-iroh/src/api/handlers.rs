//! API request handlers.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{error, info, instrument};

use super::routes::AppState;
use crate::auth::validate_ntdf_token;
use crate::profile::{ContentCategory, CreatorProfile};

// MARK: - Response Types

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishProfileResponse {
    pub success: bool,
    pub public_id: String,
    pub ticket: String,
    pub version: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchProfileResponse {
    pub profile: CreatorProfile,
    pub ticket: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchProfilesResponse {
    pub profiles: Vec<CreatorProfile>,
    pub total: usize,
    pub has_more: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

// MARK: - Query Parameters

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub categories: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct FeaturedQuery {
    pub limit: Option<usize>,
}

// MARK: - Handlers

/// Health check endpoint.
#[instrument(skip_all)]
pub async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Publish a new creator profile.
#[instrument(skip(state, headers, profile))]
pub async fn publish_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(profile): Json<CreatorProfile>,
) -> Result<Json<PublishProfileResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate authentication
    if let Err(e) = validate_auth(&headers, &state).await {
        return Err(e);
    }

    info!("Publishing profile for: {}", profile.public_id);

    match state.store.publish(&profile).await {
        Ok(ticket) => {
            info!("Profile published successfully: {}", profile.public_id);
            Ok(Json(PublishProfileResponse {
                success: true,
                public_id: profile.public_id.clone(),
                ticket,
                version: profile.version,
            }))
        }
        Err(e) => {
            error!("Failed to publish profile: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "PUBLISH_FAILED".to_string(),
                }),
            ))
        }
    }
}

/// Get a profile by public ID.
#[instrument(skip(state))]
pub async fn get_profile(
    State(state): State<AppState>,
    Path(public_id): Path<String>,
) -> Result<Json<FetchProfileResponse>, (StatusCode, Json<ErrorResponse>)> {
    let public_id_bytes = match hex::decode(&public_id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid public ID format".to_string(),
                    code: "INVALID_PUBLIC_ID".to_string(),
                }),
            ));
        }
    };

    match state.store.fetch(&public_id_bytes).await {
        Ok(Some(profile)) => {
            let entry = state.store.get_entry(&public_id_bytes).await;
            Ok(Json(FetchProfileResponse {
                profile,
                ticket: entry.map(|e| e.ticket),
            }))
        }
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Profile not found".to_string(),
                code: "PROFILE_NOT_FOUND".to_string(),
            }),
        )),
        Err(e) => {
            error!("Failed to fetch profile: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "FETCH_FAILED".to_string(),
                }),
            ))
        }
    }
}

/// Update an existing profile.
#[instrument(skip(state, headers, profile))]
pub async fn update_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(public_id): Path<String>,
    Json(mut profile): Json<CreatorProfile>,
) -> Result<Json<PublishProfileResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Validate authentication
    if let Err(e) = validate_auth(&headers, &state).await {
        return Err(e);
    }

    // Verify public ID matches
    if profile.public_id != public_id {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Public ID mismatch".to_string(),
                code: "PUBLIC_ID_MISMATCH".to_string(),
            }),
        ));
    }

    let public_id_bytes = match hex::decode(&public_id) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        }
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid public ID format".to_string(),
                    code: "INVALID_PUBLIC_ID".to_string(),
                }),
            ));
        }
    };

    // Check if profile exists
    if !state.store.has_profile(&public_id_bytes).await {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Profile not found".to_string(),
                code: "PROFILE_NOT_FOUND".to_string(),
            }),
        ));
    }

    // Update timestamps and version
    profile.updated_at = chrono::Utc::now();
    profile.version += 1;

    match state.store.update(&profile).await {
        Ok(ticket) => {
            info!("Profile updated successfully: {}", profile.public_id);
            Ok(Json(PublishProfileResponse {
                success: true,
                public_id: profile.public_id.clone(),
                ticket,
                version: profile.version,
            }))
        }
        Err(e) => {
            error!("Failed to update profile: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "UPDATE_FAILED".to_string(),
                }),
            ))
        }
    }
}

/// Get a profile by Iroh ticket.
#[instrument(skip(state))]
pub async fn get_profile_by_ticket(
    State(state): State<AppState>,
    Path(ticket): Path<String>,
) -> Result<Json<FetchProfileResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.store.fetch_by_ticket(&ticket).await {
        Ok(profile) => Ok(Json(FetchProfileResponse {
            profile,
            ticket: Some(ticket),
        })),
        Err(e) => {
            error!("Failed to fetch profile by ticket: {}", e);
            Err((
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Profile not found".to_string(),
                    code: "PROFILE_NOT_FOUND".to_string(),
                }),
            ))
        }
    }
}

/// Search profiles.
#[instrument(skip(state))]
pub async fn search_profiles(
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<SearchProfilesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let limit = params.limit.unwrap_or(20).min(100);

    let categories: Option<Vec<ContentCategory>> = params.categories.as_ref().map(|cats| {
        cats.split(',')
            .filter_map(|c| serde_json::from_str(&format!("\"{}\"", c.trim())).ok())
            .collect()
    });

    match state
        .store
        .search(params.q.as_deref(), categories.as_deref(), limit + 1)
        .await
    {
        Ok(mut profiles) => {
            let has_more = profiles.len() > limit;
            if has_more {
                profiles.truncate(limit);
            }
            let total = profiles.len();
            Ok(Json(SearchProfilesResponse {
                profiles,
                total,
                has_more,
            }))
        }
        Err(e) => {
            error!("Search failed: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "SEARCH_FAILED".to_string(),
                }),
            ))
        }
    }
}

/// Get featured profiles.
#[instrument(skip(state))]
pub async fn get_featured_profiles(
    State(state): State<AppState>,
    Query(params): Query<FeaturedQuery>,
) -> Result<Json<SearchProfilesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let limit = params.limit.unwrap_or(10).min(50);

    match state.store.get_featured(limit).await {
        Ok(profiles) => {
            let total = profiles.len();
            Ok(Json(SearchProfilesResponse {
                profiles,
                total,
                has_more: false,
            }))
        }
        Err(e) => {
            error!("Failed to get featured profiles: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                    code: "FEATURED_FAILED".to_string(),
                }),
            ))
        }
    }
}

// MARK: - Helper Functions

async fn validate_auth(
    headers: &HeaderMap,
    state: &AppState,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let auth_header = headers
        .get("Authorization")
        .and_then(|h| h.to_str().ok());

    match auth_header {
        Some(auth) if auth.starts_with("NTDF ") => {
            let token = &auth[5..];
            if let Err(e) = validate_ntdf_token(token, &state.config.ntdf_expected_audience) {
                return Err((
                    StatusCode::UNAUTHORIZED,
                    Json(ErrorResponse {
                        error: e.to_string(),
                        code: "AUTH_FAILED".to_string(),
                    }),
                ));
            }
            Ok(())
        }
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Missing or invalid Authorization header".to_string(),
                code: "AUTH_REQUIRED".to_string(),
            }),
        )),
    }
}
