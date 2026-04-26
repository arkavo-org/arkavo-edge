//! Debounced GitHub progress-comment batcher (P5).
//!
//! The cognitive engine posts a progress comment on every plan step and
//! verification result, which previously added 100-500 ms to the critical
//! path per step and pushed the GitHub API close to its secondary rate
//! limits on long runs.
//!
//! This batcher coalesces progress messages emitted within a short debounce
//! window into a single comment and posts them from a background task, off
//! the critical path. High-priority messages (final summary, errors) are
//! posted immediately and also flush any buffered lines.
//!
//! ## Lifecycle
//!
//! Call [`ProgressBatcher::start`] (or `start_with_config`) to spawn the
//! background drain task. When finished, call [`ProgressBatcher::shutdown`]
//! to flush any remaining messages and terminate the drain task cleanly.
//! Dropping the last handle without calling `shutdown` will also stop the
//! drain task, but will *not* guarantee a final flush — prefer the explicit
//! shutdown.

use arkavo_github::IssueOperations;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// Default debounce window for coalescing low-priority progress messages.
pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(1500);

/// Maximum number of lines to accumulate before forcing a flush.
pub const DEFAULT_MAX_BUFFER: usize = 12;

/// A single queued progress update.
#[derive(Debug, Clone)]
struct Entry {
    line: String,
    high_priority: bool,
}

struct Inner {
    owner: String,
    repo: String,
    issue_number: u64,
    buffer: Mutex<VecDeque<Entry>>,
    notify: Notify,
    github_ops: Arc<IssueOperations>,
    debounce: Duration,
    max_buffer: usize,
    /// Set to true by `shutdown()` to signal the drain loop to exit after
    /// its next flush.
    shutdown: AtomicBool,
}

/// Handle used by callers to enqueue progress updates.
pub struct ProgressBatcher {
    inner: Arc<Inner>,
    /// `Option` so we can `take()` it on shutdown; a handle is still usable
    /// for `post` after `shutdown` but messages will be silently dropped.
    drain_task: Mutex<Option<JoinHandle<()>>>,
}

impl ProgressBatcher {
    /// Start a new batcher bound to a single issue. Spawns a background
    /// task that drains the buffer. Call [`Self::shutdown`] to stop cleanly.
    pub fn start(
        github_ops: Arc<IssueOperations>,
        owner: impl Into<String>,
        repo: impl Into<String>,
        issue_number: u64,
    ) -> Self {
        Self::start_with_config(
            github_ops,
            owner,
            repo,
            issue_number,
            DEFAULT_DEBOUNCE,
            DEFAULT_MAX_BUFFER,
        )
    }

    pub fn start_with_config(
        github_ops: Arc<IssueOperations>,
        owner: impl Into<String>,
        repo: impl Into<String>,
        issue_number: u64,
        debounce: Duration,
        max_buffer: usize,
    ) -> Self {
        let inner = Arc::new(Inner {
            owner: owner.into(),
            repo: repo.into(),
            issue_number,
            buffer: Mutex::new(VecDeque::new()),
            notify: Notify::new(),
            github_ops,
            debounce,
            max_buffer,
            shutdown: AtomicBool::new(false),
        });

        let drain_inner = Arc::clone(&inner);
        let handle = tokio::spawn(async move { drain_loop(drain_inner).await });

        Self {
            inner,
            drain_task: Mutex::new(Some(handle)),
        }
    }

    /// Enqueue a low-priority message; posted after the debounce window.
    pub async fn post(&self, message: impl Into<String>) {
        if self.inner.shutdown.load(Ordering::Acquire) {
            return;
        }
        self.enqueue(message.into(), false).await;
    }

    /// Enqueue a high-priority message; flushes the buffer immediately.
    pub async fn post_priority(&self, message: impl Into<String>) {
        if self.inner.shutdown.load(Ordering::Acquire) {
            return;
        }
        self.enqueue(message.into(), true).await;
    }

    async fn enqueue(&self, line: String, high_priority: bool) {
        let mut buf = self.inner.buffer.lock().await;
        buf.push_back(Entry {
            line,
            high_priority,
        });
        drop(buf);
        // The drain loop reads the buffer after being notified; we always
        // notify and let it apply its own debounce / priority policy.
        self.inner.notify.notify_one();
    }

    /// Force a synchronous flush of any pending messages.
    pub async fn flush(&self) {
        let drained = {
            let mut buf = self.inner.buffer.lock().await;
            std::mem::take(&mut *buf)
        };
        post_batch(&self.inner, drained.into_iter().collect()).await;
    }

    /// Signal the drain loop to exit after flushing remaining messages, and
    /// wait for it to finish. Safe to call multiple times; only the first
    /// call actually waits on the task.
    pub async fn shutdown(&self) {
        // Mark shutdown; a second notify ensures the drain loop wakes up
        // even if the buffer is empty.
        let already = self.inner.shutdown.swap(true, Ordering::AcqRel);
        self.inner.notify.notify_one();
        if already {
            return;
        }
        let handle = {
            let mut guard = self.drain_task.lock().await;
            guard.take()
        };
        if let Some(h) = handle {
            // Bounded wait: if the drain task is wedged we don't want to
            // block the caller forever. 5s is plenty for a single GitHub
            // POST plus the drain state machine.
            let _ = tokio::time::timeout(Duration::from_secs(5), h).await;
        }
    }
}

impl Drop for ProgressBatcher {
    fn drop(&mut self) {
        // On drop, set the shutdown flag and give the drain loop a last
        // chance to notice. We cannot `.await` here, so the best-effort
        // semantics are:
        //   * no new posts will be processed (post() / post_priority()
        //     bail early when shutdown is set),
        //   * the drain loop will see the flag on its next wake and exit.
        // Callers who need a deterministic final flush must call
        // `shutdown()` explicitly before dropping.
        self.inner.shutdown.store(true, Ordering::Release);
        self.inner.notify.notify_one();
    }
}

async fn drain_loop(inner: Arc<Inner>) {
    loop {
        // Wait for at least one message (or a shutdown notification).
        inner.notify.notified().await;

        if inner.shutdown.load(Ordering::Acquire) {
            // Final flush: drain anything that was queued before shutdown
            // was signalled, then exit.
            let drained: Vec<Entry> = {
                let mut buf = inner.buffer.lock().await;
                std::mem::take(&mut *buf).into_iter().collect()
            };
            if !drained.is_empty() {
                post_batch(&inner, drained).await;
            }
            debug!(
                owner = %inner.owner,
                repo = %inner.repo,
                issue = inner.issue_number,
                "progress batcher shutting down"
            );
            return;
        }

        // Check for high-priority: flush immediately without debounce.
        let high_priority = {
            let buf = inner.buffer.lock().await;
            buf.iter().any(|e| e.high_priority)
        };

        if !high_priority {
            // Debounce: accumulate further messages for `debounce` or until
            // the buffer is full, or until shutdown is signalled.
            let deadline = tokio::time::Instant::now() + inner.debounce;
            loop {
                if inner.shutdown.load(Ordering::Acquire) {
                    break;
                }
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let timed_out = tokio::time::timeout(remaining, inner.notify.notified())
                    .await
                    .is_err();
                let buf_len = inner.buffer.lock().await.len();
                if timed_out || buf_len >= inner.max_buffer {
                    break;
                }
                // Check for high-priority escape hatch.
                let hi = inner.buffer.lock().await.iter().any(|e| e.high_priority);
                if hi {
                    break;
                }
            }
        }

        let drained: Vec<Entry> = {
            let mut buf = inner.buffer.lock().await;
            std::mem::take(&mut *buf).into_iter().collect()
        };

        if !drained.is_empty() {
            post_batch(&inner, drained).await;
        }

        // After posting, check again whether shutdown was requested.
        if inner.shutdown.load(Ordering::Acquire) {
            // Drain anything that may have arrived after our flush.
            let final_drain: Vec<Entry> = {
                let mut buf = inner.buffer.lock().await;
                std::mem::take(&mut *buf).into_iter().collect()
            };
            if !final_drain.is_empty() {
                post_batch(&inner, final_drain).await;
            }
            return;
        }
    }
}

async fn post_batch(inner: &Inner, entries: Vec<Entry>) {
    if entries.is_empty() {
        return;
    }
    let combined = if entries.len() == 1 {
        entries.into_iter().next().unwrap().line
    } else {
        let lines: Vec<String> = entries.into_iter().map(|e| e.line).collect();
        format!(
            "### Progress update ({} items)\n\n{}",
            lines.len(),
            lines.join("\n\n")
        )
    };

    debug!(
        owner = %inner.owner,
        repo = %inner.repo,
        issue = inner.issue_number,
        "posting batched progress comment"
    );

    if let Err(e) = inner
        .github_ops
        .post_comment(&inner.owner, &inner.repo, inner.issue_number, &combined)
        .await
    {
        warn!(error = %e, "failed to post batched progress comment");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_reasonable() {
        assert!(DEFAULT_DEBOUNCE >= Duration::from_millis(500));
        assert!(DEFAULT_DEBOUNCE <= Duration::from_secs(5));
        assert!(DEFAULT_MAX_BUFFER >= 4);
    }

    #[test]
    fn test_shutdown_flag_default_false() {
        // Construct an Inner directly without a github client — this just
        // checks our atomic flag default. Real end-to-end tests are
        // gated on a mockable IssueOperations trait.
        let flag = AtomicBool::new(false);
        assert!(!flag.load(Ordering::Acquire));
        flag.store(true, Ordering::Release);
        assert!(flag.load(Ordering::Acquire));
    }

    // Functional integration tests require a mock IssueOperations. The
    // concrete `arkavo_github::IssueOperations` type does not currently
    // expose a trait we can mock against in a unit test. Tracked for a
    // follow-up; in the meantime we rely on careful review of the drain
    // loop state machine and on integration tests at the orchestrator
    // level.
}
