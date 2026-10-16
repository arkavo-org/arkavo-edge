//! Answering an asset request with the generator, without the model
//! (ASSET-003).

// The message handler is this module's only caller outside `assets`; plain
// `pub` here would trip `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::sync::Arc;
use std::time::Duration;

use arkavo_image_gen::tool::JOB_DEADLINE;
use arkavo_image_gen::{ASSET_REQUEST_SCHEMA, AssetJob, AssetRequest, ImageGenError, ImageRequest};
use arkavo_protocol::types::{Message, MessagePart, MessageSendResponse, TaskError, TaskStatus};
use arkavo_tasks::task_executor::TaskExecutor;
use jsonrpsee::types::ErrorObjectOwned;
use tokio::time::Instant;
use tracing::{info, warn};

use super::{AssetService, DeliveryError, wait_on_task};

/// JSON-RPC error for a request this host refuses before opening a task,
/// so the requester can ask another member (ASSET-002).
const REFUSED: i32 = -32010;

/// Task error code of an agent that cannot generate images.
const UNAVAILABLE: &str = "IMAGE_GENERATION_UNAVAILABLE";

/// The asset request a message carries, if any.
pub(crate) fn request_in(message: &Message) -> Option<Result<AssetRequest, String>> {
    message.parts.iter().find_map(|part| match part {
        MessagePart::Data { schema, content } if schema == ASSET_REQUEST_SCHEMA => {
            Some(serde_json::from_value::<AssetRequest>(content.clone()).map_err(|e| e.to_string()))
        }
        _ => None,
    })
}

/// Take an asset request: moderated, checked against what this host can do
/// now, and handed to the generator in its own task.
pub(crate) async fn accept_request(
    assets: Option<&Arc<AssetService>>,
    router: Option<&Arc<arkavo_router::Router>>,
    task_executor: &Arc<TaskExecutor>,
    message: Message,
    request: AssetRequest,
) -> Result<MessageSendResponse, ErrorObjectOwned> {
    request.check().map_err(|e| refusal(-32602, &e))?;
    if let Some(router) = router {
        for prompt in prompts(&request.items) {
            if let Some(arkavo_router::ModerationResult::Block {
                policy_id, reason, ..
            }) = router.check_preflight(prompt)
            {
                info!(policy_id = %policy_id, "Asset request blocked by preflight policy");
                return Err(ErrorObjectOwned::owned(
                    -32001,
                    format!("Blocked by policy: {policy_id}"),
                    Some(reason),
                ));
            }
        }
    }
    let service = assets.filter(|s| s.generator.is_some());
    if let Some(service) = service {
        service.admit(&request).map_err(|e| refusal(REFUSED, &e))?;
    }
    let task_id = task_executor.submit_task(message).await.map_err(|e| {
        ErrorObjectOwned::owned(-32603, "Failed to submit task", Some(e.to_string()))
    })?;
    match service {
        Some(service) => {
            tokio::spawn(
                service
                    .clone()
                    .serve(request, task_executor.clone(), task_id),
            );
        }
        None => {
            let error = TaskError {
                code: UNAVAILABLE.to_string(),
                message: "this agent does not generate images".to_string(),
                details: None,
            };
            if let Err(e) = task_executor.fail_task(&task_id, error).await {
                warn!(%task_id, error = %e, "could not fail asset task");
            }
        }
    }
    Ok(MessageSendResponse {
        task_id: task_id.to_string(),
        status: TaskStatus::Submitted,
        response: None,
    })
}

/// One line per asset set: the files, then the model and its license.
fn describe(manifest: &arkavo_image_gen::Manifest) -> String {
    let files: Vec<String> = manifest
        .assets
        .iter()
        .map(|a| {
            let alpha = if a.has_alpha { ", alpha" } else { "" };
            format!("{} ({}x{}{alpha})", a.name, a.width, a.height)
        })
        .collect();
    format!("{}\n{}", files.join(", "), manifest.license_line())
}

fn prompts(items: &[ImageRequest]) -> impl Iterator<Item = &str> {
    items
        .iter()
        .flat_map(|i| std::iter::once(i.prompt.as_str()).chain(i.negative_prompt.as_deref()))
}

fn unavailable() -> ImageGenError {
    ImageGenError::NoCapableMember {
        what: "this host".to_string(),
    }
}

/// The error a requester matches on: the stable code, then the message.
fn refusal(code: i32, error: &ImageGenError) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(code, format!("{}: {error}", error.code()), None::<()>)
}

impl AssetService {
    /// Whether this host takes every item of `request` now.
    fn admit(&self, request: &AssetRequest) -> Result<(), ImageGenError> {
        let generator = self.generator.as_ref().ok_or_else(unavailable)?;
        request
            .items
            .iter()
            .try_for_each(|item| generator.admits(item))
    }

    /// Generate an accepted request item by item, stage the assets and
    /// complete its task with a summary and one manifest per item.
    async fn serve(
        self: Arc<Self>,
        request: AssetRequest,
        executor: Arc<TaskExecutor>,
        task_id: uuid::Uuid,
    ) {
        if let Err(e) = executor
            .update_task_status(&task_id, TaskStatus::Working)
            .await
        {
            warn!(%task_id, error = %e, "could not mark asset task working");
        }
        let requester = request
            .requester
            .as_deref()
            .unwrap_or("an unnamed requester");
        info!(%task_id, requester, items = request.items.len(), "serving asset request");
        match Box::pin(self.generate_all(&request, &executor, &task_id)).await {
            Ok(parts) => {
                let message = Message {
                    parts,
                    metadata: None,
                };
                let result = serde_json::to_value(&message).unwrap_or(serde_json::Value::Null);
                if let Err(e) = executor.complete_task(&task_id, result).await {
                    warn!(%task_id, error = %e, "could not complete asset task");
                }
            }
            Err(e) => {
                warn!(%task_id, code = e.code(), error = %e, "asset request failed");
                let error = TaskError {
                    code: e.code().to_string(),
                    message: e.to_string(),
                    details: None,
                };
                if let Err(e) = executor.fail_task(&task_id, error).await {
                    warn!(%task_id, error = %e, "could not fail asset task");
                }
            }
        }
    }

    async fn generate_all(
        &self,
        request: &AssetRequest,
        executor: &TaskExecutor,
        task_id: &uuid::Uuid,
    ) -> Result<Vec<MessagePart>, DeliveryError> {
        let generator = self.generator.clone().ok_or_else(unavailable)?;
        let until = Instant::now() + Duration::from_secs(request.deadline_secs).min(JOB_DEADLINE);
        let mut summary = Vec::new();
        let mut manifests = Vec::new();
        let items = request.items.len();
        for (index, item) in request.items.iter().enumerate() {
            let (generator, wanted) = (generator.clone(), item.clone());
            let job = AssetJob::spawn(item.clone(), move |tx| async move {
                let report = |p| {
                    tx.send_replace(Some(p));
                };
                generator.generate(&wanted, until, &report).await
            });
            let prefix = if items > 1 {
                format!("item {} of {items}: ", index + 1)
            } else {
                String::new()
            };
            let generated = wait_on_task(&job, until, executor, task_id, &prefix).await?;
            summary.push(describe(&generated.manifest));
            manifests.push(self.manifest_part(&generated).await?);
        }
        let mut parts = vec![MessagePart::Text {
            content: summary.join("\n"),
        }];
        parts.extend(manifests);
        Ok(parts)
    }
}

#[cfg(test)]
mod tests;
