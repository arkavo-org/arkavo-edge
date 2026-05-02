//! Abstraction over "how do we deliver progress updates to the user".
//!
//! Two implementations:
//!
//! * **Direct** (default) — posts each comment synchronously in the
//!   critical path. This was the default behavior before Wave 2.
//! * **Batched** — hands each comment to a background
//!   [`ProgressBatcher`] that debounces and coalesces into fewer
//!   GitHub API calls. Enabled via the `ARKAVO_PROGRESS_BATCHER`
//!   environment variable.
//!
//! The sink is intended to be created per [`CognitiveEngine::execute`]
//! call so a long-lived engine can still handle many issues concurrently,
//! each with its own batcher.
//!
//! A sink is cheap to clone; both variants are internally `Arc`-backed.

use crate::error::Result;
use crate::progress_batcher::ProgressBatcher;
use arkavo_github::IssueOperations;
use std::sync::Arc;
use tracing::debug;

/// Environment variable that enables the batched progress sink when set
/// to a truthy value (`1`, `true`, `yes`, `on`, case-insensitive).
pub const ENV_BATCHER_ENABLED: &str = "ARKAVO_PROGRESS_BATCHER";

/// Priority hint for a single progress message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressPriority {
    /// Routine progress; may be batched / debounced.
    Normal,
    /// High-priority (final summary, errors); should flush immediately.
    High,
}

#[derive(Clone)]
enum Backend {
    Direct {
        github_ops: Arc<IssueOperations>,
        owner: String,
        repo: String,
        issue_number: u64,
    },
    Batched {
        batcher: Arc<ProgressBatcher>,
    },
}

/// Clonable handle for delivering progress messages. Dropping the last
/// clone does not implicitly flush a batched sink; callers who need a
/// deterministic final flush must call [`ProgressSink::shutdown`].
#[derive(Clone)]
pub struct ProgressSink {
    backend: Backend,
}

impl ProgressSink {
    /// Build a sink for a single execute() call. Consults
    /// [`ENV_BATCHER_ENABLED`] to decide between direct and batched mode.
    pub fn for_issue(
        github_ops: Arc<IssueOperations>,
        owner: impl Into<String>,
        repo: impl Into<String>,
        issue_number: u64,
    ) -> Self {
        let owner = owner.into();
        let repo = repo.into();
        if batcher_enabled() {
            debug!(
                owner = %owner,
                repo = %repo,
                issue = issue_number,
                "progress sink: batched"
            );
            let batcher = Arc::new(ProgressBatcher::start(
                Arc::clone(&github_ops),
                owner.clone(),
                repo.clone(),
                issue_number,
            ));
            Self {
                backend: Backend::Batched { batcher },
            }
        } else {
            Self {
                backend: Backend::Direct {
                    github_ops,
                    owner,
                    repo,
                    issue_number,
                },
            }
        }
    }

    /// Force a direct (non-batched) sink. Useful for tests and for
    /// callers that want to bypass the env var.
    pub fn direct(
        github_ops: Arc<IssueOperations>,
        owner: impl Into<String>,
        repo: impl Into<String>,
        issue_number: u64,
    ) -> Self {
        Self {
            backend: Backend::Direct {
                github_ops,
                owner: owner.into(),
                repo: repo.into(),
                issue_number,
            },
        }
    }

    pub async fn post(&self, priority: ProgressPriority, message: &str) -> Result<()> {
        match &self.backend {
            Backend::Direct {
                github_ops,
                owner,
                repo,
                issue_number,
            } => {
                github_ops
                    .post_comment(owner, repo, *issue_number, message)
                    .await?;
                Ok(())
            }
            Backend::Batched { batcher } => {
                match priority {
                    ProgressPriority::Normal => batcher.post(message).await,
                    ProgressPriority::High => batcher.post_priority(message).await,
                }
                Ok(())
            }
        }
    }

    /// Flush any pending messages and release resources. For the
    /// batched sink this waits (with a 5s bound) for the drain task
    /// to exit; for the direct sink this is a no-op.
    pub async fn shutdown(&self) {
        if let Backend::Batched { batcher } = &self.backend {
            batcher.shutdown().await;
        }
    }
}

fn batcher_enabled() -> bool {
    match std::env::var(ENV_BATCHER_ENABLED) {
        Ok(v) => {
            let lower = v.to_lowercase();
            matches!(lower.as_str(), "1" | "true" | "yes" | "on")
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batcher_flag_parsing() {
        // Setting env vars in tests requires single-threaded access;
        // `cargo test` runs this suite with --test-threads=1 in CI
        // for env-sensitive modules like this one. Here we just
        // verify the parse logic on a scoped flag.
        let original = std::env::var(ENV_BATCHER_ENABLED).ok();
        // SAFETY: env var mutation is only safe in single-threaded
        // contexts. We restore the original at the end.
        unsafe {
            std::env::remove_var(ENV_BATCHER_ENABLED);
        }
        assert!(!batcher_enabled());
        unsafe {
            std::env::set_var(ENV_BATCHER_ENABLED, "1");
        }
        assert!(batcher_enabled());
        unsafe {
            std::env::set_var(ENV_BATCHER_ENABLED, "true");
        }
        assert!(batcher_enabled());
        unsafe {
            std::env::set_var(ENV_BATCHER_ENABLED, "off");
        }
        assert!(!batcher_enabled());
        // Restore
        match original {
            Some(v) => unsafe {
                std::env::set_var(ENV_BATCHER_ENABLED, v);
            },
            None => unsafe {
                std::env::remove_var(ENV_BATCHER_ENABLED);
            },
        }
    }
}
