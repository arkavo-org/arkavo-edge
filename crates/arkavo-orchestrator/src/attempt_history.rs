//! Reflexion-style failure memory for plan retries.
//!
//! Captures a compact, natural-language summary of why a prior attempt
//! failed so that subsequent planning calls can avoid repeating the same
//! mistakes. In-memory only; cleared once an issue completes successfully.
//!
//! ## Bounded growth
//!
//! To prevent unbounded memory growth on a runaway retry loop, per-issue
//! history is capped at [`MAX_ATTEMPTS_PER_ISSUE`] entries. When the cap
//! is reached, the oldest entry is dropped so the most recent context is
//! always preserved.

use crate::cognitive_engine_core::VerificationResult;
use crate::text_utils::truncate_ellipsis;
use std::collections::HashMap;
use std::sync::RwLock;

/// Maximum attempts retained per issue; beyond this, records are FIFO-evicted.
///
/// Chosen so that a realistic 3-retry outer loop still fits, with headroom
/// for adjustment sub-steps, without the map growing without bound on a
/// misbehaving caller.
pub const MAX_ATTEMPTS_PER_ISSUE: usize = 8;

/// Classification of why a prior attempt failed. Used to render a more
/// informative prompt hint to the next planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureKind {
    /// Plan failed pre-flight contract validation (R5).
    PlanInvalid,
    /// A step's wall-clock time exceeded the configured timeout (R4).
    Timeout,
    /// A command inside a step returned an execution error.
    CommandError,
    /// Verification checks returned non-passing results.
    VerificationFailed,
    /// Budget exhausted before plan completed.
    BudgetExceeded,
    /// Generic / unclassified failure.
    Other,
}

impl FailureKind {
    fn label(&self) -> &'static str {
        match self {
            Self::PlanInvalid => "plan-invalid",
            Self::Timeout => "timeout",
            Self::CommandError => "command-error",
            Self::VerificationFailed => "verification-failed",
            Self::BudgetExceeded => "budget-exceeded",
            Self::Other => "other",
        }
    }
}

/// A single recorded attempt against an issue.
#[derive(Debug, Clone)]
pub struct AttemptRecord {
    pub attempt_number: u32,
    pub steps_completed: usize,
    pub total_steps: usize,
    pub failure_summary: String,
    pub failed_verifications: Vec<String>,
    pub kind: FailureKind,
}

impl AttemptRecord {
    /// Render this record as a prompt-friendly block for the next planner.
    pub fn to_prompt_fragment(&self) -> String {
        let mut out = format!(
            "Attempt {} [{}] ({}/{} steps): {}",
            self.attempt_number,
            self.kind.label(),
            self.steps_completed,
            self.total_steps,
            self.failure_summary
        );
        if !self.failed_verifications.is_empty() {
            out.push_str("\n  Verification failures:");
            for v in self.failed_verifications.iter().take(3) {
                use std::fmt::Write as _;
                let _ = write!(out, "\n    - {v}");
            }
        }
        out
    }
}

/// Thread-safe per-issue attempt history store.
#[derive(Default)]
pub struct AttemptHistory {
    /// Key: "owner/repo#issue_number"
    inner: RwLock<HashMap<String, Vec<AttemptRecord>>>,
}

impl AttemptHistory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn key(repository: &str, issue_number: u64) -> String {
        format!("{repository}#{issue_number}")
    }

    /// Record a failed attempt with an explicit classification.
    ///
    /// Enforces a [`MAX_ATTEMPTS_PER_ISSUE`] cap via FIFO eviction so that
    /// a runaway retry loop cannot exhaust memory.
    // Each argument carries distinct, non-derivable context (who, what,
    // which kind of failure, progress, verification state, human summary).
    // Collapsing them into a single struct would only add indirection.
    #[allow(clippy::too_many_arguments)]
    pub fn record_failure_kind(
        &self,
        repository: &str,
        issue_number: u64,
        kind: FailureKind,
        steps_completed: usize,
        total_steps: usize,
        verification_results: &[VerificationResult],
        failure_summary: impl Into<String>,
    ) {
        let key = Self::key(repository, issue_number);
        let failed: Vec<String> = verification_results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| format!("{:?}: {}", r.check, truncate_ellipsis(&r.details, 200)))
            .collect();

        let attempt_number = {
            let guard = self.inner.read().expect("attempt history poisoned");
            guard.get(&key).map_or(1, |v| v.len() as u32 + 1)
        };

        let record = AttemptRecord {
            attempt_number,
            steps_completed,
            total_steps,
            failure_summary: failure_summary.into(),
            failed_verifications: failed,
            kind,
        };

        let mut guard = self.inner.write().expect("attempt history poisoned");
        let entries = guard.entry(key).or_default();
        entries.push(record);
        // FIFO-evict oldest if we exceed the cap.
        if entries.len() > MAX_ATTEMPTS_PER_ISSUE {
            let excess = entries.len() - MAX_ATTEMPTS_PER_ISSUE;
            entries.drain(0..excess);
        }
    }

    /// Backward-compatible shim for callers that don't classify failures.
    /// New code should prefer [`Self::record_failure_kind`].
    pub fn record_failure(
        &self,
        repository: &str,
        issue_number: u64,
        steps_completed: usize,
        total_steps: usize,
        verification_results: &[VerificationResult],
        failure_summary: impl Into<String>,
    ) {
        self.record_failure_kind(
            repository,
            issue_number,
            FailureKind::Other,
            steps_completed,
            total_steps,
            verification_results,
            failure_summary,
        );
    }

    /// Get all prior attempts for an issue.
    pub fn get(&self, repository: &str, issue_number: u64) -> Vec<AttemptRecord> {
        let key = Self::key(repository, issue_number);
        self.inner
            .read()
            .expect("attempt history poisoned")
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }

    /// Clear the history for an issue (on successful completion).
    pub fn clear(&self, repository: &str, issue_number: u64) {
        let key = Self::key(repository, issue_number);
        self.inner
            .write()
            .expect("attempt history poisoned")
            .remove(&key);
    }

    /// Build a prompt fragment summarizing prior failed attempts for this
    /// issue. Returns `None` if there are no prior attempts.
    pub fn to_prompt_block(&self, repository: &str, issue_number: u64) -> Option<String> {
        let attempts = self.get(repository, issue_number);
        if attempts.is_empty() {
            return None;
        }
        let mut block = String::from(
            "Prior attempts on this issue failed. Learn from these failures and avoid repeating them:\n",
        );
        for a in &attempts {
            block.push_str(&a.to_prompt_fragment());
            block.push('\n');
        }
        block.push_str(
            "\nProduce a plan that explicitly addresses the root causes of the above failures.",
        );
        Some(block)
    }

    /// Total number of attempt records currently stored (diagnostic).
    pub fn total_records(&self) -> usize {
        self.inner
            .read()
            .expect("attempt history poisoned")
            .values()
            .map(|v| v.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cognitive_engine_core::VerificationCheck;
    use arkavo_test_macros::spec;

    fn failed_result(detail: &str) -> VerificationResult {
        VerificationResult {
            check: VerificationCheck::TestsPassing,
            passed: false,
            details: detail.to_string(),
        }
    }

    #[spec("ORCH-005")]
    #[test]
    fn test_empty_history_returns_none() {
        let h = AttemptHistory::new();
        assert!(h.to_prompt_block("owner/repo", 1).is_none());
        assert!(h.get("owner/repo", 1).is_empty());
    }

    #[spec("ORCH-005")]
    #[test]
    fn test_record_and_retrieve() {
        let h = AttemptHistory::new();
        h.record_failure(
            "owner/repo",
            42,
            2,
            4,
            &[failed_result("expected: foo, got: bar")],
            "step 3 failed",
        );
        let block = h.to_prompt_block("owner/repo", 42).unwrap();
        assert!(block.contains("Attempt 1"));
        assert!(block.contains("2/4 steps"));
        assert!(block.contains("step 3 failed"));
    }

    #[spec("ORCH-005")]
    #[test]
    fn test_classified_failure_renders_label() {
        let h = AttemptHistory::new();
        h.record_failure_kind(
            "owner/repo",
            7,
            FailureKind::Timeout,
            1,
            3,
            &[],
            "step 2 hit timeout",
        );
        let block = h.to_prompt_block("owner/repo", 7).unwrap();
        assert!(block.contains("[timeout]"));
        assert!(block.contains("step 2 hit timeout"));
    }

    #[spec("ORCH-005")]
    #[test]
    fn test_multiple_attempts_increment() {
        let h = AttemptHistory::new();
        for _ in 0..3 {
            h.record_failure("owner/repo", 1, 0, 3, &[], "boom");
        }
        let attempts = h.get("owner/repo", 1);
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts[0].attempt_number, 1);
        assert_eq!(attempts[2].attempt_number, 3);
    }

    #[spec("ORCH-005")]
    #[test]
    fn test_clear_removes_history() {
        let h = AttemptHistory::new();
        h.record_failure("owner/repo", 1, 0, 1, &[], "nope");
        assert!(h.to_prompt_block("owner/repo", 1).is_some());
        h.clear("owner/repo", 1);
        assert!(h.to_prompt_block("owner/repo", 1).is_none());
    }

    #[spec("ORCH-005")]
    #[test]
    fn test_bounded_cap_fifo_evicts() {
        let h = AttemptHistory::new();
        // Insert more than the cap; oldest entries must be evicted.
        for i in 0..(MAX_ATTEMPTS_PER_ISSUE + 3) {
            h.record_failure("owner/repo", 1, 0, 1, &[], format!("failure {i}"));
        }
        let attempts = h.get("owner/repo", 1);
        assert_eq!(attempts.len(), MAX_ATTEMPTS_PER_ISSUE);
        // The oldest entries (0, 1, 2) should have been evicted; entry "failure 3" should be first.
        assert!(
            attempts
                .first()
                .unwrap()
                .failure_summary
                .contains("failure 3")
        );
        assert!(
            attempts
                .last()
                .unwrap()
                .failure_summary
                .contains(&format!("failure {}", MAX_ATTEMPTS_PER_ISSUE + 2))
        );
    }

    #[spec("ORCH-005")]
    #[test]
    fn test_total_records_tracks_sum() {
        let h = AttemptHistory::new();
        h.record_failure("a/b", 1, 0, 1, &[], "x");
        h.record_failure("a/b", 2, 0, 1, &[], "y");
        h.record_failure("c/d", 1, 0, 1, &[], "z");
        assert_eq!(h.total_records(), 3);
    }
}
