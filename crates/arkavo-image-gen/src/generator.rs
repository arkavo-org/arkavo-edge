//! The generator a qualifying agent runs (IMG-005, IMG-011, IMG-012,
//! ASSET-001).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{OnceCell, Semaphore};
use tokio::time::Instant;

use crate::args::{ModelPaths, sd_args, write_prompts};
use crate::catalog::{ALIASES, RUNTIME, RuntimePin, WeightSet};
use crate::descriptor::{Descriptor, ModelRow};
use crate::error::{ImageGenError, Result};
use crate::fetch::Fetcher;
use crate::host::Host;
use crate::lock::{self, HostLock};
use crate::output::{Generated, Producer, assets_dir, collect};
use crate::plan::{BASE_PIXELS, Offer, offers};
use crate::progress::{Event, Progress};
use crate::request::{ImageRequest, Job};
use crate::run::{RunSpec, Sandbox, run};
use crate::runtime::{Runtime, install};
use crate::store::Store;
use fetches::InFlight;

/// Requests that may wait behind the one running (IMG-011).
pub const MAX_QUEUE: usize = 4;

/// Image generation on this host.
pub struct Generator {
    host: Host,
    store: Store,
    fetcher: Fetcher,
    runtime_pin: &'static RuntimePin,
    sandbox: Sandbox,
    offers: Vec<Offer>,
    producer: Producer,
    runtime: OnceCell<Runtime>,
    fetch_lock: Arc<tokio::sync::Mutex<()>>,
    fetches: Mutex<HashMap<usize, InFlight>>,
    gate: Semaphore,
    waiting: AtomicUsize,
    speeds: Mutex<HashMap<&'static str, f64>>,
}

impl Generator {
    /// The generator for this machine, or `None` when it does not qualify
    /// (IMG-001). Prunes runtimes no pin of this release names (IMG-020).
    pub fn for_this_host(resident_chat_bytes: u64, producer: Producer) -> Option<Self> {
        let host = Host::detect();
        if !host.qualifies() {
            return None;
        }
        let offers = offers(host.budget(resident_chat_bytes));
        if offers.is_empty() {
            return None;
        }
        let store = Store::new(Store::default_root()?);
        if let Err(e) = store.prune(RUNTIME.tag) {
            tracing::warn!(error = %e, "could not prune image-gen files");
        }
        let generator = Self::new(host, store, offers, &RUNTIME, Sandbox::MacOs).ok()?;
        Some(generator.with_producer(producer))
    }

    /// A generator with explicit parts, for hosts and runtimes chosen by the
    /// caller.
    pub fn new(
        host: Host,
        store: Store,
        offers: Vec<Offer>,
        runtime_pin: &'static RuntimePin,
        sandbox: Sandbox,
    ) -> Result<Self> {
        Ok(Self {
            host,
            store,
            fetcher: Fetcher::new()?,
            runtime_pin,
            sandbox,
            offers,
            producer: Producer::default(),
            runtime: OnceCell::new(),
            fetch_lock: Arc::new(tokio::sync::Mutex::new(())),
            fetches: Mutex::new(HashMap::new()),
            gate: Semaphore::new(1),
            waiting: AtomicUsize::new(0),
            speeds: Mutex::new(HashMap::new()),
        })
    }

    /// Name the agent that produces assets, for their manifests.
    #[must_use]
    pub fn with_producer(mut self, producer: Producer) -> Self {
        self.producer = producer;
        self
    }

    pub fn offers(&self) -> &[Offer] {
        &self.offers
    }

    pub fn offer(&self, entry_id: &str) -> Option<&Offer> {
        self.offers.iter().find(|o| o.entry.id == entry_id)
    }

    /// What members are told about this host (ASSET-001).
    pub fn descriptor(&self) -> Descriptor {
        let speeds = lock(&self.speeds);
        Descriptor {
            chip: self.host.chip.clone(),
            unified_memory_gib: self.host.memory_bytes as f64 / f64::from(1u32 << 30),
            runtime_build: self.runtime_pin.build,
            queue_depth: self.waiting.load(Ordering::Relaxed),
            models: self
                .offers
                .iter()
                .map(|offer| {
                    let remaining = self.store.missing_bytes(offer.set);
                    ModelRow {
                        id: offer.entry.id.to_string(),
                        aliases: ALIASES
                            .iter()
                            .filter(|(_, id)| *id == offer.entry.id)
                            .map(|(alias, _)| (*alias).to_string())
                            .collect(),
                        weight_set: offer.set.label,
                        ready: remaining == 0,
                        bytes_remaining: remaining,
                        largest_pixels: offer.largest_pixels(false).unwrap_or(0),
                        largest_pixels_guided: offer.largest_pixels(true),
                        steps: offer.entry.profile.steps,
                        seconds_per_step: speeds.get(offer.entry.id).copied(),
                    }
                })
                .collect(),
        }
    }

    /// Fetch the runtime and an entry's weights ahead of the first request.
    pub async fn prefetch(
        &self,
        model: &str,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<()> {
        let job = ImageRequest {
            prompt: "prefetch".into(),
            model: Some(model.to_string()),
            ..ImageRequest::default()
        }
        .validate()?;
        let offer = *self.offer(job.entry.id).ok_or(ImageGenError::NotOffered {
            entry: job.entry.id,
        })?;
        self.runtime(progress).await?;
        let far = Instant::now() + std::time::Duration::from_hours(7 * 24);
        self.models(offer.set, far, progress).await.map(drop)
    }

    /// Whether this host would take `request` now: valid, offered, within
    /// its budget, and room in the queue. A requester that hears no can try
    /// another member before anything has started (ASSET-002).
    pub fn admits(&self, request: &ImageRequest) -> Result<()> {
        let job = request.validate()?;
        let offer = self.offer(job.entry.id).ok_or(ImageGenError::NotOffered {
            entry: job.entry.id,
        })?;
        offer.check(job.pixels(), job.guided())?;
        let depth = self.waiting.load(Ordering::SeqCst);
        if depth >= MAX_QUEUE {
            return Err(ImageGenError::Busy { depth });
        }
        Ok(())
    }

    /// Generate `request` here, by `deadline`.
    pub async fn generate(
        &self,
        request: &ImageRequest,
        deadline: Instant,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<Generated> {
        // Boxed: fetching, queueing and the run make a future of tens of
        // kilobytes that would otherwise sit in every caller's.
        Box::pin(self.generate_inner(request, deadline, progress)).await
    }

    async fn generate_inner(
        &self,
        request: &ImageRequest,
        deadline: Instant,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<Generated> {
        let job = request.validate()?;
        let offer = *self.offer(job.entry.id).ok_or(ImageGenError::NotOffered {
            entry: job.entry.id,
        })?;
        offer.check(job.pixels(), job.guided())?;
        let admitted = Admission::enter(&self.waiting)?;
        let runtime = self.runtime(progress).await?.clone();
        let models = self.models(offer.set, deadline, progress).await?;
        if self.gate.available_permits() == 0 {
            progress(Progress::Queued);
        }
        let _permit = tokio::time::timeout_at(deadline, self.gate.acquire())
            .await
            .map_err(|_| ImageGenError::TimedOut)?
            .map_err(|_| ImageGenError::Canceled)?;
        drop(admitted);
        // Other agents on this host take turns with this one (IMG-011).
        let _host = HostLock::acquire(self.store.root(), lock::GENERATE, Some(deadline), || {
            progress(Progress::Queued);
        })
        .await?;
        runtime.verify().await?;
        // A directory of this run's own: another agent's start or run on
        // this host never touches it.
        let dir = assets_dir("arkavo-image-gen-")?;
        let mut result = Box::pin(self.run_job(
            &job, offer.set, &runtime, &models, &dir, deadline, progress, false,
        ))
        .await;
        // A GPU that ran out of memory left garbage where the image should
        // be. Decoding in tiles needs a fraction of that memory, so the job
        // runs once more that way before the requester hears of a failure.
        if matches!(result, Err(ImageGenError::GpuFailed { .. })) {
            tracing::warn!("the GPU failed generating; trying again with tiled decoding");
            result = Box::pin(self.run_job(
                &job, offer.set, &runtime, &models, &dir, deadline, progress, true,
            ))
            .await;
        }
        if result.is_err() {
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_job(
        &self,
        job: &Job,
        set: &'static WeightSet,
        runtime: &Runtime,
        models: &ModelPaths,
        dir: &Path,
        deadline: Instant,
        progress: &(dyn Fn(Progress) + Send + Sync),
        low_memory: bool,
    ) -> Result<Generated> {
        let prompts = write_prompts(job, dir).await?;
        let image = AtomicU32::new(1);
        let on_event = |event: Event| match event {
            Event::ImageStarted { image: i, .. } => image.store(i, Ordering::Relaxed),
            Event::Step { step, steps } => progress(Progress::Step {
                image: image.load(Ordering::Relaxed),
                images: job.count,
                step,
                steps,
            }),
            Event::GpuFailed => {}
        };
        let spec = RunSpec {
            cli: runtime.cli(),
            args: sd_args(job, models, &prompts, dir, low_memory),
            out_dir: dir,
            steps: job.entry.profile.steps,
            deadline,
            sandbox: self.sandbox,
        };
        let output = run(spec, &on_event).await;
        let _ = tokio::fs::remove_file(&prompts.prompt).await;
        if let Some(negative) = &prompts.negative {
            let _ = tokio::fs::remove_file(negative).await;
        }
        let output = output?;
        if let Some(seconds) = output.seconds_per_step {
            self.record_speed(job, seconds);
        }
        let mut manifest = collect(dir, job, set, &output.seeds).await?;
        manifest.producer.clone_from(&self.producer.did);
        manifest.producer_agent.clone_from(&self.producer.agent_id);
        Ok(Generated::owning(dir.to_path_buf(), manifest))
    }

    /// Keep a moving average of seconds per step, normalized to 1 MP so
    /// rows compare across request sizes.
    fn record_speed(&self, job: &Job, seconds: f64) {
        let normalized = seconds * BASE_PIXELS as f64 / job.pixels() as f64;
        lock(&self.speeds)
            .entry(job.entry.id)
            .and_modify(|speed| *speed = 0.7f64.mul_add(*speed, 0.3 * normalized))
            .or_insert(normalized);
    }

    async fn runtime(&self, progress: &(dyn Fn(Progress) + Send + Sync)) -> Result<&Runtime> {
        self.runtime
            .get_or_try_init(|| async {
                let queued = || progress(Progress::Queued);
                let _fetching = take_turn(&self.fetch_lock, queued).await;
                let _host = HostLock::acquire(self.store.root(), lock::FETCH, None, queued).await?;
                install(&self.store, &self.fetcher, self.runtime_pin).await
            })
            .await
    }
}

/// A request counted as waiting until it starts running or leaves.
struct Admission<'a>(&'a AtomicUsize);

impl<'a> Admission<'a> {
    fn enter(waiting: &'a AtomicUsize) -> Result<Self> {
        let depth = waiting.fetch_add(1, Ordering::SeqCst);
        if depth >= MAX_QUEUE {
            waiting.fetch_sub(1, Ordering::SeqCst);
            return Err(ImageGenError::Busy { depth });
        }
        Ok(Self(waiting))
    }
}

impl Drop for Admission<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// This process's turn at `mutex`, calling `waiting` first if another
/// request has it.
async fn take_turn(
    mutex: &tokio::sync::Mutex<()>,
    waiting: impl FnOnce(),
) -> tokio::sync::MutexGuard<'_, ()> {
    if let Ok(guard) = mutex.try_lock() {
        return guard;
    }
    waiting();
    mutex.lock().await
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

mod fetches;

#[cfg(test)]
mod tests;
