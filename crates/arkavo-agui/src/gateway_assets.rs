//! Images a task's agent delivered, fetched by the process that runs the
//! gateway and served to the browser with the task they belong to.
//!
//! An agent answers an image request with text and a manifest whose assets
//! sit on its data plane (ASSET-008). Fetching them needs that data plane
//! and the manifest checks agents use (ASSET-007), which live outside this
//! crate, so whoever runs the gateway hands it an [`AssetFetcher`]. The
//! gateway serves only the files a fetch recorded on a task, never a path
//! a request names.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use arkavo_protocol::types::MessagePart;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::gateway::{AppState, ConnectionInfo, TrackedTask};
use crate::gateway_routing::broadcast_event;
use crate::types::AgUiEvent;

/// The schema of the data part that lists an answer's assets (ASSET-008).
const MANIFEST_SCHEMA: &str = "urn:arkavo:asset:manifest:v1";

/// An image as the browser shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskAsset {
    pub name: String,
    pub url: String,
    pub width: u32,
    pub height: u32,
    #[serde(rename = "hasAlpha")]
    pub has_alpha: bool,
    pub bytes: u64,
    /// Who generated it, with which model, under which license (IMG-013).
    pub license: String,
}

/// One image a fetch wrote and verified.
#[derive(Debug, Clone)]
pub struct FetchedAsset {
    pub name: String,
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub bytes: u64,
    pub license: String,
}

/// An image fetched for a task: what the browser is told, and the file it
/// is served from.
#[derive(Debug, Clone)]
pub struct StoredAsset {
    pub asset: TaskAsset,
    pub path: PathBuf,
}

impl StoredAsset {
    fn new(task_id: &str, fetched: FetchedAsset) -> Self {
        Self {
            asset: TaskAsset {
                url: format!("/assets/{}/{}", url_safe(task_id), url_safe(&fetched.name)),
                name: fetched.name,
                width: fetched.width,
                height: fetched.height,
                has_alpha: fetched.has_alpha,
                bytes: fetched.bytes,
                license: fetched.license,
            },
            path: fetched.path,
        }
    }
}

/// Fetches and verifies the assets an agent's answer references.
#[async_trait::async_trait]
pub trait AssetFetcher: Send + Sync {
    /// Fetch what `parts` reference, for the gateway's task `task_id`.
    async fn fetch(
        &self,
        task_id: &str,
        parts: &[MessagePart],
    ) -> Result<Vec<FetchedAsset>, String>;
}

/// Whether an answer lists assets on the agent's data plane.
pub fn references_assets(parts: &[MessagePart]) -> bool {
    parts
        .iter()
        .any(|part| matches!(part, MessagePart::Data { schema, .. } if schema == MANIFEST_SCHEMA))
}

/// Fetch the assets a completed task's answer references, record them on
/// the task and tell every open view. Without a fetcher the views are told
/// the images stayed on the agent, rather than shown nothing.
pub(crate) async fn deliver(
    fetcher: Option<&Arc<dyn AssetFetcher>>,
    task_id: &str,
    parts: &[MessagePart],
    task_store: &Arc<RwLock<HashMap<String, TrackedTask>>>,
    connections: &Arc<RwLock<HashMap<String, ConnectionInfo>>>,
) {
    if !references_assets(parts) {
        return;
    }
    let Some(fetcher) = fetcher else {
        note(
            task_id,
            "Images stayed on the agent: this build cannot fetch them",
            task_store,
            connections,
        )
        .await;
        return;
    };
    note(
        task_id,
        "Fetching images from the agent",
        task_store,
        connections,
    )
    .await;
    match fetcher.fetch(task_id, parts).await {
        Ok(fetched) => {
            let stored: Vec<_> = fetched
                .into_iter()
                .map(|asset| StoredAsset::new(task_id, asset))
                .collect();
            if let Some(task) = task_store.write().await.get_mut(task_id) {
                task.assets = stored;
                task.asset_note = None;
            }
        }
        Err(why) => {
            tracing::warn!(task_id, "fetching the task's images failed: {why}");
            if let Some(task) = task_store.write().await.get_mut(task_id) {
                task.asset_note = Some(format!("Images could not be fetched: {why}"));
            }
        }
    }
    announce(task_id, task_store, connections).await;
}

async fn note(
    task_id: &str,
    text: &str,
    task_store: &Arc<RwLock<HashMap<String, TrackedTask>>>,
    connections: &Arc<RwLock<HashMap<String, ConnectionInfo>>>,
) {
    if let Some(task) = task_store.write().await.get_mut(task_id) {
        task.asset_note = Some(text.to_string());
    }
    announce(task_id, task_store, connections).await;
}

/// Send every open view the task as it now stands, its output included,
/// so the update replaces nothing it already shows.
async fn announce(
    task_id: &str,
    task_store: &Arc<RwLock<HashMap<String, TrackedTask>>>,
    connections: &Arc<RwLock<HashMap<String, ConnectionInfo>>>,
) {
    let Some(task) = task_store.read().await.get(task_id).cloned() else {
        return;
    };
    let event = AgUiEvent::TaskStatusChanged {
        task_id: task.id.clone(),
        status: task.status.clone(),
        progress: task.progress,
        result: task.result.clone(),
        metrics: None,
        assets: task
            .assets
            .iter()
            .map(|stored| stored.asset.clone())
            .collect(),
        asset_note: task.asset_note.clone(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    broadcast_event(&event, connections).await;
}

/// Serve one image a fetch recorded on a task.
pub async fn asset_handler(
    State(state): State<AppState>,
    Path((task_id, name)): Path<(String, String)>,
) -> Response {
    let path = state
        .task_store
        .read()
        .await
        .get(&task_id)
        .and_then(|task| {
            task.assets
                .iter()
                .find(|stored| stored.asset.name == name)
                .map(|stored| stored.path.clone())
        });
    let Some(path) = path else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::CACHE_CONTROL, "private, max-age=86400"),
            ],
            bytes,
        )
            .into_response(),
        Err(e) => {
            tracing::warn!(path = %path.display(), "a recorded image is unreadable: {e}");
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

/// `text` with everything but letters, digits, `.`, `-` and `_` percent
/// encoded, for one segment of a URL path.
fn url_safe(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
