//! Weight fetches shared by every request waiting on the same set, and
//! running on past a requester that gives up (IMG-005, IMG-012).

use std::sync::Arc;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use tokio::sync::watch;
use tokio::time::Instant;

use super::{Generator, lock, take_turn};
use crate::args::ModelPaths;
use crate::catalog::WeightSet;
use crate::error::{ImageGenError, Result};
use crate::lock::HostLock;
use crate::progress::Progress;

type FetchResult = std::result::Result<ModelPaths, Arc<ImageGenError>>;

/// A fetch of one weight set, as every request waiting on it sees it.
pub(super) struct InFlight {
    result: Shared<BoxFuture<'static, FetchResult>>,
    progress: watch::Receiver<Progress>,
}

impl Generator {
    /// The set's files, fetching them in a task that outlives a requester
    /// that gives up, shared with every request waiting on the same set.
    pub(super) async fn models(
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
                        progress(watched.borrow_and_update().clone());
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
        watch::Receiver<Progress>,
    ) {
        let mut fetches = lock(&self.fetches);
        let in_flight = fetches.entry(key).or_insert_with(|| self.spawn_fetch(set));
        let joined = (in_flight.result.clone(), in_flight.progress.clone());
        drop(fetches);
        joined
    }

    fn spawn_fetch(&self, set: &'static WeightSet) -> InFlight {
        let needed = self.store.missing_bytes(set);
        let (tx, rx) = watch::channel(Progress::Fetching { fetched: 0, needed });
        let (store, lock) = (self.store.clone(), self.fetch_lock.clone());
        let task = tokio::spawn(async move {
            // Another fetch here, or another agent's on this host, goes first.
            let queued = || {
                tx.send_replace(Progress::Queued);
            };
            let _fetching = take_turn(&lock, queued).await;
            let _host = HostLock::acquire(store.root(), crate::lock::FETCH, None, queued)
                .await
                .map_err(Arc::new)?;
            store
                .fetch_set(set, &|fetched, needed| {
                    tx.send_replace(Progress::Fetching { fetched, needed });
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
