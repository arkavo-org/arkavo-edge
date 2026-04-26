//! Debounced GitHub progress-comment batcher (P5).
//!
//! The cognitive engine posts a progress comment on every plan step and
//! verification result, which previously added 100\u2013500 ms to the critical
//! path per step and pushed the GitHub API close to its secondary rate
//! limits on long runs.
//!
//! This batcher coalesces progress messages emitted within a short debounce
//! window into a single comment and posts them from a background task, off
//! the critical path. High-priority messages (final summary, errors) are
//! posted immediately and also flush any buffered lines.

use arkavo_github::IssueOperations;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};
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
}

/// Handle used by callers to enqueue progress updates.
#[derive(Clone)]
pub struct ProgressBatcher {
    inner: Arc<Inner>,
}

impl ProgressBatcher {
    /// Start a new batcher bound to a single issue. Spawns a background
    /// task that drains the buffer. Drop the returned handle to stop
    /// accepting new messages; call `flush` before dropping to ensure
    /// outstanding messages are posted.
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
        });

        let drain_inner = Arc::clone(&inner);
        tokio::spawn(async move { drain_loop(drain_inner).await });

        Self { inner }
    }

    /// Enqueue a low-priority message; posted after the debounce window.
    pub async fn post(&self, message: impl Into<String>) {
        self.enqueue(message.into(), false).await;
    }

    /// Enqueue a high-priority message; flushes the buffer immediately.
    pub async fn post_priority(&self, message: impl Into<String>) {
        self.enqueue(message.into(), true).await;
    }

    async fn enqueue(&self, line: String, high_priority: bool) {
        let mut buf = self.inner.buffer.lock().await;
        buf.push_back(Entry {
            line,
            high_priority,
        });
        let over_cap = buf.len() >= self.inner.max_buffer;
        drop(buf);
        if high_priority || over_cap {
            self.inner.notify.notify_one();
        } else {
            // Still notify so the drain loop starts its debounce timer.
            self.inner.notify.notify_one();
        }
    }

    /// Force a synchronous flush of any pending messages.
    pub async fn flush(&self) {
        let drained = {
            let mut buf = self.inner.buffer.lock().await;
            std::mem::take(&mut *buf)
        };
        post_batch(&self.inner, drained.into_iter().collect()).await;
    }
}

async fn drain_loop(inner: Arc<Inner>) {
    loop {
        // Wait for at least one message.
        inner.notify.notified().await;

        // Check for high-priority: flush immediately.
        let high_priority = {
            let buf = inner.buffer.lock().await;
            buf.iter().any(|e| e.high_priority)
        };

        if !high_priority {
            // Debounce: accumulate further messages for `debounce` or until
            // the buffer is full.
            let deadline = tokio::time::Instant::now() + inner.debounce;
            loop {
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
                let hi = inner
                    .buffer
                    .lock()
                    .await
                    .iter()
                    .any(|e| e.high_priority);
                if hi {
                    break;
                }
            }
        }

        let drained: Vec<Entry> = {
            let mut buf = inner.buffer.lock().await;
            std::mem::take(&mut *buf).into_iter().collect()
        };

        if drained.is_empty() {
            continue;
        }

        post_batch(&inner, drained).await;
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
        format!("### Progress update ({} items)\n\n{}", lines.len(), lines.join("\n\n"))
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

    // Functional integration tests would require a mock IssueOperations;
    // the GitHub client type doesn't expose a trait to mock against, so
    // we rely on the unit correctness of the drain_loop state machine
    // (implicit through careful code review) and system-level tests
    // outside this crate.
}