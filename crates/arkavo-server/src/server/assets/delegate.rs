//! Generation on a capable member (ASSET-002, ASSET-006, ASSET-009).
//!
//! The member is chosen from the descriptors members report, sent one
//! asset request, and followed by its task's progress until the deadline.
//! Its assets are fetched over the data plane, checked against its
//! manifest, and handed on as local files, so this agent stages them for
//! its own requester under its own tickets.

use std::sync::Arc;
use std::time::Duration;

use arkavo_image_gen::catalog;
use arkavo_image_gen::tool::Delegate;
use arkavo_image_gen::transfer::{FetchPolicy, fetch};
use arkavo_image_gen::{
    ASSET_MANIFEST_SCHEMA, ASSET_REQUEST_SCHEMA, AssetRequest, Delivery, Descriptor, Generated,
    ImageGenError, ImageRequest, Need, Progress, assets_dir, rank,
};
use arkavo_mcp_mesh::{Member, MeshToolsState, Patience, Peer};
use arkavo_protocol::types::{Message, MessagePart, TaskGetResponse, TaskStatus};
use arkavo_tdf_iroh::IrohTransport;
use async_trait::async_trait;
use tokio::time::Instant;
use tracing::info;

use super::Identity;

/// A member whose progress does not change for this long is stalled.
const STALL: Duration = Duration::from_mins(5);

/// Longest wait between two looks at the member's task.
const MAX_POLL: Duration = Duration::from_secs(5);

const PATIENCE: Patience = Patience {
    timeout_ms: 30_000,
    max_retries: 1,
};

type Result<T> = std::result::Result<T, ImageGenError>;

pub(super) struct MeshDelegate {
    mesh: Arc<MeshToolsState>,
    identity: Identity,
    transport: IrohTransport,
    fetch_policy: FetchPolicy,
    stall: Duration,
}

impl MeshDelegate {
    pub(super) fn new(
        mesh: Arc<MeshToolsState>,
        identity: Identity,
        transport: IrohTransport,
    ) -> Self {
        Self {
            mesh,
            identity,
            transport,
            fetch_policy: FetchPolicy::default(),
            stall: STALL,
        }
    }

    /// The same delegate, impatient enough for a test.
    #[cfg(test)]
    pub(super) const fn with_patience(
        mut self,
        fetch_policy: FetchPolicy,
        stall: Duration,
    ) -> Self {
        self.fetch_policy = fetch_policy;
        self.stall = stall;
        self
    }

    /// Members that report image generation, other than this agent.
    async fn capable(&self) -> Vec<(Member, Descriptor)> {
        self.mesh
            .member_reports()
            .await
            .into_iter()
            .filter(|m| m.agent_id != self.identity.agent_id)
            .filter_map(|m| {
                let descriptor = serde_json::from_value(m.report.image_generation.clone()?).ok()?;
                Some((m, descriptor))
            })
            .collect()
    }

    /// Offer `request` to `member`; the task id once it accepts.
    async fn offer(
        &self,
        member: &Member,
        request: &ImageRequest,
        deadline: Instant,
    ) -> std::result::Result<(Peer, String), String> {
        let asked = AssetRequest {
            items: vec![request.clone()],
            deadline_secs: deadline.saturating_duration_since(Instant::now()).as_secs(),
            requester: self.identity.did.clone(),
        };
        let content = serde_json::to_value(&asked).map_err(|e| e.to_string())?;
        let message = Message {
            parts: vec![
                MessagePart::Text {
                    content: format!("Asset request from {}", self.identity.agent_id),
                },
                MessagePart::Data {
                    schema: ASSET_REQUEST_SCHEMA.to_string(),
                    content,
                },
            ],
            metadata: self
                .identity
                .did
                .as_ref()
                .map(|did| serde_json::json!({ "sender_did": did })),
        };
        let peer = Peer::connect(&member.address, &member.agent_id, PATIENCE)
            .await
            .map_err(|e| e.to_string())?;
        match peer.send_message(message).await {
            Ok(accepted) => Ok((peer, accepted.task_id)),
            Err(e) => {
                peer.close().await;
                Err(e.to_string())
            }
        }
    }

    /// Follow the member's task to its end, relaying its progress.
    async fn follow(
        &self,
        peer: &Peer,
        member: &str,
        task_id: &str,
        deadline: Instant,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<TaskGetResponse> {
        let mut last: Option<(TaskStatus, Option<String>)> = None;
        let mut changed_at = Instant::now();
        let mut wait = Duration::from_millis(250);
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(ImageGenError::TimedOut);
            }
            tokio::time::sleep(wait.min(deadline - now)).await;
            wait = (wait * 2).min(MAX_POLL);
            let seen = match peer.get_task(task_id).await {
                Ok(task) => match task.status {
                    TaskStatus::Completed => return Ok(task),
                    TaskStatus::Submitted | TaskStatus::Working => {
                        let message = task.progress.and_then(|p| p.message);
                        Some((task.status, message))
                    }
                    other => return Err(ended(member, other, &task)),
                },
                // A member that stops answering is judged like one whose
                // progress stopped.
                Err(e) => {
                    tracing::debug!(member, error = %e, "member did not answer a poll");
                    last.clone()
                }
            };
            if seen == last {
                if changed_at.elapsed() >= self.stall {
                    return Err(ImageGenError::Stalled {
                        member: member.to_string(),
                        minutes: self.stall.as_secs().div_ceil(60),
                    });
                }
                continue;
            }
            if let Some((_, Some(message))) = &seen {
                progress(Progress::Relayed {
                    member: member.to_string(),
                    message: message.clone(),
                });
            }
            last = seen;
            changed_at = Instant::now();
        }
    }

    /// Fetch and check the assets a finished task references.
    async fn take_delivery(&self, member: &str, task: TaskGetResponse) -> Result<Generated> {
        let delivery: Delivery = task
            .result
            .iter()
            .flat_map(|m| m.parts.iter())
            .find_map(|part| match part {
                MessagePart::Data { schema, content } if schema == ASSET_MANIFEST_SCHEMA => {
                    serde_json::from_value(content.clone()).ok()
                }
                _ => None,
            })
            .ok_or_else(|| {
                ImageGenError::Delegation(format!("{member} finished without an asset manifest"))
            })?;
        // Fetched files are untrusted until checked, so they get a directory
        // of their own, owned from here and removed on every early return.
        let dir = assets_dir("arkavo-received-")?;
        let mut generated = Generated::owning(dir.clone(), delivery.manifest.clone());
        let fetched = fetch(&self.transport, &delivery, &dir, self.fetch_policy).await;
        if let Some((name, e)) = fetched.failed.first() {
            return Err(ImageGenError::Delegation(format!("{member}: {name}: {e}")));
        }
        if fetched.saved.is_empty() {
            return Err(ImageGenError::Delegation(format!(
                "{member} delivered no assets"
            )));
        }
        generated.manifest.assets = fetched.saved.into_iter().map(|(entry, _)| entry).collect();
        Ok(generated)
    }
}

/// Why a member's task ended without assets.
fn ended(member: &str, status: TaskStatus, task: &TaskGetResponse) -> ImageGenError {
    let reason = task.error.as_ref().map_or_else(
        || format!("task ended {status:?}"),
        |e| format!("{}: {}", e.code, e.message),
    );
    ImageGenError::Delegation(format!("{member}: {reason}"))
}

/// Why no member was asked: none offers the model, or none at this size.
fn none_capable(members: &[(Member, Descriptor)], need: &Need) -> ImageGenError {
    let named = |d: &Descriptor| {
        d.models
            .iter()
            .filter(|row| row.id == need.model || row.aliases.contains(&need.model))
            .map(|row| {
                if need.guided {
                    row.largest_pixels_guided.unwrap_or(0)
                } else {
                    row.largest_pixels
                }
            })
            .max()
    };
    let largest = members.iter().filter_map(|(_, d)| named(d)).max();
    match (largest, catalog::resolve(&need.model)) {
        (Some(largest), Some(resolved)) => ImageGenError::TooLargeForHost {
            entry: resolved.entry.id,
            pixels: need.pixels.unwrap_or(0),
            guided: if need.guided { " with guidance" } else { "" },
            largest: format!("{largest} pixels on any member"),
        },
        _ => ImageGenError::NoCapableMember {
            what: need.model.clone(),
        },
    }
}

#[async_trait]
impl Delegate for MeshDelegate {
    async fn generate(
        &self,
        request: &ImageRequest,
        deadline: Instant,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<Generated> {
        let need = Need::of(request);
        let members = self.capable().await;
        let unmet = none_capable(&members, &need);
        let ranked = rank(members, &need);
        if ranked.is_empty() {
            return Err(unmet);
        }
        let mut refusals = Vec::new();
        for member in ranked {
            match self.offer(&member, request, deadline).await {
                Ok((peer, task_id)) => {
                    info!(member = %member.agent_id, %task_id, "member accepted asset request");
                    let followed = self
                        .follow(&peer, &member.agent_id, &task_id, deadline, progress)
                        .await;
                    peer.close().await;
                    return self.take_delivery(&member.agent_id, followed?).await;
                }
                Err(reason) => refusals.push(format!("{} refused: {reason}", member.agent_id)),
            }
        }
        Err(ImageGenError::NoCapableMember {
            what: format!("{} ({})", need.model, refusals.join("; ")),
        })
    }
}

#[cfg(test)]
mod tests;
