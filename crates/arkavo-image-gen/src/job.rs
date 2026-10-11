//! Generations that run in their own task, so the reply that delivers them
//! can wait past the model's turn (ASSET-008).

use std::future::Future;
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::error::{ImageGenError, Result};
use crate::output::Generated;
use crate::progress::Progress;
use crate::request::ImageRequest;

type Outcome = std::result::Result<Generated, Arc<ImageGenError>>;

/// A generation in flight, shared by the tool call that started it and the
/// reply that delivers it.
#[derive(Clone)]
pub struct AssetJob {
    request: ImageRequest,
    result: Shared<BoxFuture<'static, Outcome>>,
    progress: watch::Receiver<Option<Progress>>,
}

/// Where a job sends progress while it runs.
pub type ProgressSender = watch::Sender<Option<Progress>>;

impl AssetJob {
    /// Run `work` in its own task.
    pub fn spawn<F, Fut>(request: ImageRequest, work: F) -> Self
    where
        F: FnOnce(ProgressSender) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Generated>> + Send + 'static,
    {
        let (tx, rx) = watch::channel(None);
        let task = tokio::spawn(async move { work(tx).await.map_err(Arc::new) });
        let result = async move {
            task.await.unwrap_or_else(|e| {
                Err(Arc::new(ImageGenError::Output(format!(
                    "generation task ended: {e}"
                ))))
            })
        };
        Self {
            request,
            result: result.boxed().shared(),
            progress: rx,
        }
    }

    pub const fn request(&self) -> &ImageRequest {
        &self.request
    }

    /// The latest progress the job reported.
    pub fn progress(&self) -> Option<Progress> {
        self.progress.borrow().clone()
    }

    /// The outcome, if the job finishes by `until`.
    pub async fn wait_until(&self, until: Instant) -> Option<Outcome> {
        tokio::time::timeout_at(until, self.result.clone())
            .await
            .ok()
    }

    /// Wait for the outcome, calling `on_progress` as the job reports.
    pub async fn wait_reporting(
        &self,
        until: Instant,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Outcome {
        let mut progress = self.progress.clone();
        let mut result = std::pin::pin!(self.result.clone());
        loop {
            tokio::select! {
                outcome = &mut result => return outcome,
                changed = progress.changed() => {
                    if changed.is_ok()
                        && let Some(p) = progress.borrow_and_update().clone()
                    {
                        on_progress(p);
                    }
                }
                () = tokio::time::sleep_until(until) => return Err(Arc::new(ImageGenError::TimedOut)),
            }
        }
    }
}

impl PartialEq for AssetJob {
    /// The same job, as two holders of one generation see it.
    fn eq(&self, other: &Self) -> bool {
        self.result.ptr_eq(&other.result)
    }
}

impl Eq for AssetJob {}

impl std::fmt::Debug for AssetJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetJob")
            .field("prompt", &self.request.prompt)
            .field("progress", &self.progress())
            .finish_non_exhaustive()
    }
}

/// Takes the jobs a tool call starts, for the reply to deliver.
pub trait AssetSink: Send + Sync {
    fn accept(&self, job: AssetJob);
}

/// The jobs started while one answer was being produced.
#[derive(Default)]
pub struct Outbox {
    jobs: Mutex<Vec<AssetJob>>,
}

impl Outbox {
    /// Take every job so far, leaving the outbox empty for the next answer.
    pub fn take(&self) -> Vec<AssetJob> {
        std::mem::take(
            &mut *self
                .jobs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

impl AssetSink for Outbox {
    fn accept(&self, job: AssetJob) {
        self.jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(job);
    }
}

#[cfg(test)]
mod tests {
    // `#[tokio::test]` expands to `Runtime::block_on`, which the workspace
    // lint set disallows in library code.
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use crate::output::Manifest;
    use std::time::Duration;

    fn manifest() -> Manifest {
        serde_json::from_value(serde_json::json!({
            "model": {"id": "m", "repo": "r", "revision": "v", "diffusion": "d", "text_encoder": "t"},
            "license": "l", "license_url": "u", "runtime": "rt", "prompt": "p", "assets": []
        }))
        .unwrap()
    }

    /// Assets in a fresh directory of their own.
    fn generated() -> Generated {
        let dir = std::env::temp_dir().join(format!("arkavo-job-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        Generated::owning(dir, manifest())
    }

    fn request() -> ImageRequest {
        ImageRequest {
            prompt: "x".into(),
            ..ImageRequest::default()
        }
    }

    #[tokio::test]
    async fn jobs_report_progress_and_finish() {
        let job = AssetJob::spawn(request(), |tx| async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            tx.send_replace(Some(Progress::Queued));
            tokio::time::sleep(Duration::from_millis(20)).await;
            Ok(generated())
        });
        assert!(job.wait_until(Instant::now()).await.is_none());
        let seen = Mutex::new(Vec::new());
        let outcome = job
            .wait_reporting(Instant::now() + Duration::from_secs(5), &|p| {
                seen.lock().unwrap().push(p);
            })
            .await;
        assert!(outcome.is_ok());
        assert_eq!(seen.into_inner().unwrap(), [Progress::Queued]);
        assert_eq!(job.progress(), Some(Progress::Queued));
        assert_eq!(job.request().prompt, "x");
        // A finished job can be waited on again by another holder.
        assert!(
            job.clone()
                .wait_until(Instant::now() + Duration::from_secs(1))
                .await
                .is_some()
        );
    }

    #[arkavo_test_macros::spec("IMG-012")]
    #[tokio::test]
    async fn waits_stop_at_their_deadline_and_errors_are_shared() {
        let slow = AssetJob::spawn(request(), |_| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(generated())
        });
        let outcome = slow
            .wait_reporting(Instant::now() + Duration::from_millis(50), &|_| {})
            .await;
        assert!(matches!(
            outcome.unwrap_err().as_ref(),
            ImageGenError::TimedOut
        ));
        let failing = AssetJob::spawn(request(), |_| async move {
            Err(ImageGenError::Busy { depth: 4 })
        });
        let err = failing
            .wait_until(Instant::now() + Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(err.code(), "busy");
    }

    /// ASSET-008: the files of a job nobody will deliver are deleted once
    /// the last holder is gone, and not before.
    #[arkavo_test_macros::spec("ASSET-008")]
    #[tokio::test]
    async fn output_lives_as_long_as_a_holder_of_the_job() {
        let job = AssetJob::spawn(request(), |_| async move { Ok(generated()) });
        let other = AssetJob::spawn(request(), |_| async move { Ok(generated()) });
        assert_eq!(job, job.clone());
        assert_ne!(job, other);
        assert!(format!("{job:?}").contains("AssetJob"));

        let holder = job.clone();
        let dir = job
            .wait_until(Instant::now() + Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap()
            .dir;
        drop(job);
        assert!(dir.exists(), "another holder may still deliver it");
        drop(holder);
        assert!(!dir.exists());
    }

    #[test]
    fn outboxes_hand_over_their_jobs_once() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let outbox = Outbox::default();
        outbox.accept(AssetJob::spawn(
            request(),
            |_| async move { Ok(generated()) },
        ));
        outbox.accept(AssetJob::spawn(
            request(),
            |_| async move { Ok(generated()) },
        ));
        assert_eq!(outbox.take().len(), 2);
        assert!(outbox.take().is_empty());
    }
}
