//! Image assets: generated on this host or by a capable member, and
//! delivered to requesters over the data plane (swarm-assets.spec.yaml).
//!
//! The control plane carries requests and references only. A reply that
//! delivers assets stages each one on the agent's long-lived Iroh node and
//! lists the references in a manifest data part after its text.

use std::sync::Arc;
use std::time::Duration;

use arkavo_image_gen::tool::{Delegate, JOB_DEADLINE};
use arkavo_image_gen::{AssetJob, AssetSink, Descriptor, Generated, Generator, ImageGenError};
use arkavo_mcp_tools::ToolRegistry;
use arkavo_protocol::types::{MessagePart, TaskProgress};
use arkavo_tasks::task_executor::TaskExecutor;
use arkavo_tdf_iroh::{IrohNode, IrohTransport};
use tokio::time::Instant;
use tracing::{info, warn};

mod delegate;
mod delivery;
mod serve;

pub use delivery::{Delivery, DeliveryError, Fetched, deliveries_in, stage, take};
pub(crate) use serve::{accept_request, request_in};

/// How often a waiting reply copies a job's progress onto its task.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);

/// Who this agent is, for the members it asks and the manifests it writes.
#[derive(Debug, Clone)]
pub struct Identity {
    /// The agent id members know it by, so it never asks itself.
    pub agent_id: String,
    /// Its did:key, named as producer and as requester.
    pub did: Option<String>,
}

/// Generation for this agent, here or on members, and delivery of what it
/// produces.
pub struct AssetService {
    generator: Option<Arc<Generator>>,
    delegate: Option<Arc<dyn Delegate>>,
    transport: IrohTransport,
}

impl AssetService {
    /// The service for an agent with a data plane: it generates here when
    /// the host qualifies (IMG-001) and asks members otherwise. Without a
    /// data plane nothing could be delivered, so there is no service.
    pub async fn for_agent(
        node: Option<Arc<IrohNode>>,
        mesh: Option<Arc<arkavo_mcp_mesh::MeshToolsState>>,
        identity: Identity,
        resident_chat_bytes: u64,
    ) -> Option<Arc<Self>> {
        let node = node?;
        let did = identity.did.clone();
        let generator = tokio::task::spawn_blocking(move || {
            Generator::for_this_host(resident_chat_bytes, did).map(Arc::new)
        })
        .await
        .ok()
        .flatten();
        match &generator {
            Some(g) => info!(
                models = g.offers().len(),
                "image generation offered on this host"
            ),
            None => info!("this host does not generate images; members are asked instead"),
        }
        let transport = IrohTransport::new(node);
        let delegate = mesh.map(|mesh| {
            Arc::new(delegate::MeshDelegate::new(
                mesh,
                identity,
                transport.clone(),
            )) as Arc<dyn Delegate>
        });
        Some(Arc::new(Self::new(generator, delegate, transport)))
    }

    pub fn new(
        generator: Option<Arc<Generator>>,
        delegate: Option<Arc<dyn Delegate>>,
        transport: IrohTransport,
    ) -> Self {
        Self {
            generator,
            delegate,
            transport,
        }
    }

    /// What members are told about this host's generation (ASSET-001);
    /// `None` when it does not generate.
    pub fn descriptor(&self) -> Option<Descriptor> {
        self.generator.as_ref().map(|g| g.descriptor())
    }

    /// Offer generate_image to a model whose answer is delivered with what
    /// `sink` collects (IMG-014).
    pub fn register_tools(&self, registry: &mut ToolRegistry, sink: Arc<dyn AssetSink>) {
        if self.generator.is_some() || self.delegate.is_some() {
            arkavo_image_gen::register_tools(
                registry,
                self.generator.clone(),
                self.delegate.clone(),
                sink,
            );
        }
    }

    /// Stage `generated` on the data plane and describe it as a manifest
    /// data part.
    async fn manifest_part(&self, generated: &Generated) -> Result<MessagePart, DeliveryError> {
        stage(&self.transport, generated).await?.part()
    }
}

/// The generations a reply delivers, with the service that stages them
/// (ASSET-008). Empty for a reply that produced none.
#[derive(Clone, Default)]
pub struct Assets(Option<(Arc<AssetService>, Vec<AssetJob>)>);

impl Assets {
    pub(crate) fn of(service: &Arc<AssetService>, jobs: Vec<AssetJob>) -> Self {
        if jobs.is_empty() {
            Self::default()
        } else {
            Self(Some((service.clone(), jobs)))
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Wait for each generation, copying its progress onto the task, and
    /// stage what it produced. A generation that fails is reported in a
    /// text part, so the reply never claims files it does not carry.
    pub(crate) async fn parts(
        &self,
        executor: &TaskExecutor,
        task_id: &uuid::Uuid,
    ) -> Vec<MessagePart> {
        let Some((service, jobs)) = &self.0 else {
            return Vec::new();
        };
        let until = Instant::now() + JOB_DEADLINE;
        let mut parts = Vec::new();
        for job in jobs {
            let staged = match wait_on_task(job, until, executor, task_id, "").await {
                Ok(generated) => service.manifest_part(&generated).await,
                Err(e) => Err(e.into()),
            };
            match staged {
                Ok(part) => parts.push(part),
                Err(e) => {
                    warn!(%task_id, code = e.code(), error = %e, "assets not delivered");
                    parts.push(MessagePart::Text {
                        content: format!(
                            "Images for \"{}\" were not delivered: {}: {e}",
                            job.request().prompt,
                            e.code()
                        ),
                    });
                }
            }
        }
        parts
    }
}

impl PartialEq for Assets {
    fn eq(&self, other: &Self) -> bool {
        let jobs = |a: &Self| a.0.as_ref().map(|(_, jobs)| jobs.clone());
        jobs(self) == jobs(other)
    }
}

impl Eq for Assets {}

impl std::fmt::Debug for Assets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let jobs = self.0.as_ref().map_or(&[][..], |(_, jobs)| jobs.as_slice());
        f.debug_tuple("Assets").field(&jobs).finish()
    }
}

/// The outcome of `job` by `until`, with its progress copied onto the task
/// the requester polls, each message after `prefix`.
async fn wait_on_task(
    job: &AssetJob,
    until: Instant,
    executor: &TaskExecutor,
    task_id: &uuid::Uuid,
    prefix: &str,
) -> Result<Generated, ImageGenError> {
    let mut reported = None;
    loop {
        let step = (Instant::now() + PROGRESS_INTERVAL).min(until);
        if let Some(outcome) = job.wait_until(step).await {
            return outcome.map_err(|e| owned(&e));
        }
        if Instant::now() >= until {
            return Err(ImageGenError::TimedOut);
        }
        let progress = job.progress();
        if progress != reported {
            if let Some(p) = &progress {
                let update = TaskProgress {
                    percentage: None,
                    message: Some(format!("{prefix}{p}")),
                    eta_seconds: None,
                };
                if let Err(e) = executor.update_task_progress(task_id, update).await {
                    warn!(%task_id, error = %e, "could not report asset progress");
                }
            }
            reported = progress;
        }
    }
}

/// A shared error as one the caller owns, keeping its code and message.
fn owned(error: &ImageGenError) -> ImageGenError {
    match error {
        ImageGenError::TimedOut => ImageGenError::TimedOut,
        ImageGenError::Canceled => ImageGenError::Canceled,
        other => ImageGenError::Shared {
            code: other.code(),
            message: other.to_string(),
        },
    }
}

#[cfg(test)]
// `#[tokio::test]` expands to `Runtime::block_on`, which the crate's lint set
// disallows in library code. Same waiver as the other async test modules here.
#[allow(clippy::disallowed_methods)]
mod tests;
