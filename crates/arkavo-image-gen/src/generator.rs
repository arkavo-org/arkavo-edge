//! The generator a qualifying agent runs (IMG-005, IMG-011, IMG-012,
//! ASSET-001).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use tokio::sync::{OnceCell, Semaphore, watch};
use tokio::time::Instant;

use crate::args::{ModelPaths, sd_args, write_prompts};
use crate::catalog::{ALIASES, RUNTIME, RuntimePin, WeightSet, pinned_store_names};
use crate::descriptor::{Descriptor, ModelRow};
use crate::error::{ImageGenError, Result};
use crate::fetch::Fetcher;
use crate::host::Host;
use crate::output::{Generated, collect};
use crate::plan::{BASE_PIXELS, Offer, offers};
use crate::progress::{Event, Progress};
use crate::request::{ImageRequest, Job};
use crate::run::{RunSpec, Sandbox, run};
use crate::runtime::{Runtime, install};
use crate::store::Store;

/// Requests that may wait behind the one running (IMG-011).
pub const MAX_QUEUE: usize = 4;

type FetchResult = std::result::Result<ModelPaths, Arc<ImageGenError>>;

struct InFlight {
    result: Shared<BoxFuture<'static, FetchResult>>,
    progress: watch::Receiver<(u64, u64)>,
}

/// Image generation on this host.
pub struct Generator {
    host: Host,
    store: Store,
    fetcher: Fetcher,
    runtime_pin: &'static RuntimePin,
    sandbox: Sandbox,
    offers: Vec<Offer>,
    producer: Option<String>,
    runtime: OnceCell<Runtime>,
    fetch_lock: Arc<tokio::sync::Mutex<()>>,
    fetches: Mutex<HashMap<usize, InFlight>>,
    gate: Semaphore,
    waiting: AtomicUsize,
    speeds: Mutex<HashMap<&'static str, f64>>,
}

impl Generator {
    /// The generator for this machine, or `None` when it does not qualify
    /// (IMG-001). Prunes files no pin of this release names (IMG-020).
    pub fn for_this_host(resident_chat_bytes: u64, producer: Option<String>) -> Option<Self> {
        let host = Host::detect();
        if !host.qualifies() {
            return None;
        }
        let offers = offers(host.budget(resident_chat_bytes));
        if offers.is_empty() {
            return None;
        }
        let store = Store::new(Store::default_root()?);
        if let Err(e) = store.prune(&pinned_store_names(), RUNTIME.tag) {
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
            producer: None,
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
    pub fn with_producer(mut self, producer: Option<String>) -> Self {
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
        self.runtime().await?;
        let far = Instant::now() + std::time::Duration::from_hours(7 * 24);
        self.models(offer.set, far, progress).await.map(drop)
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
        let runtime = self.runtime().await?.clone();
        let models = self.models(offer.set, deadline, progress).await?;
        if self.gate.available_permits() == 0 {
            progress(Progress::Queued);
        }
        let _permit = tokio::time::timeout_at(deadline, self.gate.acquire())
            .await
            .map_err(|_| ImageGenError::TimedOut)?
            .map_err(|_| ImageGenError::Canceled)?;
        drop(admitted);
        runtime.verify().await?;
        let dir = self
            .store
            .outputs_dir()
            .join(format!("{:016x}", rand::random::<u64>()));
        tokio::fs::create_dir_all(&dir).await?;
        let result =
            Box::pin(self.run_job(&job, offer.set, &runtime, &models, &dir, deadline, progress))
                .await;
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
        };
        let spec = RunSpec {
            cli: runtime.cli(),
            args: sd_args(job, models, &prompts, dir),
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
        manifest.producer.clone_from(&self.producer);
        Ok(Generated {
            dir: dir.to_path_buf(),
            manifest,
        })
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

    async fn runtime(&self) -> Result<&Runtime> {
        self.runtime
            .get_or_try_init(|| async {
                let _fetching = self.fetch_lock.lock().await;
                install(&self.store, &self.fetcher, self.runtime_pin).await
            })
            .await
    }

    /// The set's files, fetching them in a task that outlives a requester
    /// that gives up, shared with every request waiting on the same set.
    async fn models(
        &self,
        set: &'static WeightSet,
        deadline: Instant,
        progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<ModelPaths> {
        let key = std::ptr::from_ref(set) as usize;
        let (result, mut watched) = self.join_fetch(key, set);
        let mut result = std::pin::pin!(result);
        let outcome = loop {
            tokio::select! {
                outcome = &mut result => break outcome,
                changed = watched.changed() => {
                    if changed.is_ok() {
                        let (fetched, needed) = *watched.borrow_and_update();
                        progress(Progress::Fetching { fetched, needed });
                    }
                }
                () = tokio::time::sleep_until(deadline) => return Err(ImageGenError::TimedOut),
            }
        };
        if outcome.is_err() {
            lock(&self.fetches).remove(&key);
        }
        outcome.map_err(|e| ImageGenError::Download {
            url: String::new(),
            reason: e.to_string(),
        })
    }

    /// The in-flight fetch of `set`, started if there is none. Kept out of
    /// the async caller so the map's guard never spans an await.
    fn join_fetch(
        &self,
        key: usize,
        set: &'static WeightSet,
    ) -> (
        Shared<BoxFuture<'static, FetchResult>>,
        watch::Receiver<(u64, u64)>,
    ) {
        let mut fetches = lock(&self.fetches);
        let in_flight = fetches.entry(key).or_insert_with(|| self.spawn_fetch(set));
        let joined = (in_flight.result.clone(), in_flight.progress.clone());
        drop(fetches);
        joined
    }

    fn spawn_fetch(&self, set: &'static WeightSet) -> InFlight {
        let (tx, rx) = watch::channel((0, self.store.missing_bytes(set)));
        let (store, fetcher, lock) = (
            self.store.clone(),
            self.fetcher.clone(),
            self.fetch_lock.clone(),
        );
        let task = tokio::spawn(async move {
            let _fetching = lock.lock().await;
            store
                .fetch_set(&fetcher, set, &|fetched, needed| {
                    tx.send_replace((fetched, needed));
                })
                .await
                .map_err(Arc::new)
        });
        let result = async move {
            task.await.unwrap_or_else(|e| {
                Err(Arc::new(ImageGenError::Download {
                    url: String::new(),
                    reason: format!("fetch task ended: {e}"),
                }))
            })
        };
        InFlight {
            result: result.boxed().shared(),
            progress: rx,
        }
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

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
