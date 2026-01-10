use crate::error::Result;
use async_trait::async_trait;

/// Trait for handling GitHub issues discovered during polling.
///
/// This trait allows arkavo-github to delegate issue processing to an
/// external handler (like Orchestrator) without creating a circular dependency.
#[async_trait]
pub trait IssueHandler: Send + Sync {
    /// Handle a discovered GitHub issue
    async fn handle_issue(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        title: &str,
        body: &str,
    ) -> Result<()>;
}
