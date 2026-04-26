//! Reflexion-style failure memory for plan retries.
//!
//! Captures a compact, natural-language summary of why a prior attempt
//! failed so that subsequent planning calls can avoid repeating the same
//! mistakes. In-memory only; cleared once an issue completes.

use crate::cognitive_engine_core::VerificationResult;
use std::collections::HashMap;
use std::sync::RwLock;

/// A single recorded attempt against an issue.
#[derive(Debug, Clone)]
pub struct AttemptRecord {
    pub attempt_number: u32,
    pub steps_completed: usize,
    pub total_steps: usize,
    pub failure_summary: String,
    pub failed_verifications: Vec<String>,
}

impl AttemptRecord {
    /// Render this record as a prompt-friendly block for the next planner.
    pub fn to_prompt_fragment(&self) -> String {
        let mut out = format!(
            "Attempt {} ({}/{} steps): {}",
            self.attempt_number,
            self.steps_completed,
            self.total_steps,
            self.failure_summary
        );
        if !self.failed_verifications.is_empty() {
            out.push_str("\n  Verification failures:");
            for v in self.failed_verifications.iter().take(3) {
                out.push_str(&format!("\n    - {v}"));
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

    /// Record a failed attempt.
    pub fn record_failure(
        &self,
        repository: &str,
        issue_number: u64,
        steps_completed: usize,
        total_steps: usize,
        verification_results: &[VerificationResult],
        failure_summary: impl Into<String>,
    ) {
        let key = Self::key(repository, issue_number);
        let failed: Vec<String> = verification_results
            .iter()
            .filter(|r| !r.passed)
            .map(|r| format!("{:?}: {}", r.check, truncate(&r.details, 200)))
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
        };

        let mut guard = self.inner.write().expect("attempt history poisoned");
        guard.entry(key).or_default().push(record);
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
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        // Avoid slicing a UTF-8 char boundary.
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\u{2026}", &s[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cognitive_engine_core::VerificationCheck;

    fn failed_result(detail: &str) -> VerificationResult {
        VerificationResult {
            check: VerificationCheck::TestsPassing,
            passed: false,
            details: detail.to_string(),
        }
    }

    #[test]
    fn test_empty_history_returns_none() {
        let h = AttemptHistory::new();
        assert!(h.to_prompt_block("owner/repo", 1).is_none());
        assert!(h.get("owner/repo", 1).is_empty());
    }

    #[test]
    fn test_record_and_retrieve() {
        let h = AttemptHistory::new();
        h.record_failure(
            "owner/repo",
            42,
            2,
            5,
            &[failed_result("test_foo panicked")],
            "execution aborted after step 2",
        );
        let block = h.to_prompt_block("owner/repo", 42).expect("has block");
        assert!(block.contains("Attempt 1"));
        assert!(block.contains("2/5 steps"));
        assert!(block.contains("test_foo panicked"));
        assert!(block.contains("aborted after step 2"));
    }

    #[test]
    fn test_multiple_attempts_increment() {
        let h = AttemptHistory::new();
        h.record_failure("owner/repo", 1, 1, 3, &[], "first");
        h.record_failure("owner/repo", 1, 2, 3, &[], "second");
        let all = h.get("owner/repo", 1);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].attempt_number, 1);
        assert_eq!(all[1].attempt_number, 2);
    }

    #[test]
    fn test_clear_removes_history() {
        let h = AttemptHistory::new();
        h.record_failure("owner/repo", 7, 0, 1, &[], "boom");
        h.clear("owner/repo", 7);
        assert!(h.get("owner/repo", 7).is_empty());
    }

    #[test]
    fn test_truncate_unicode_safe() {
        // Make sure we don't panic on multi-byte boundaries.
        let s = "a".repeat(100) + "日本語";
        let t = truncate(&s, 101);
        assert!(t.ends_with('\u{2026}'));
    }

    #[test]
    fn test_separate_issues_isolated() {
        let h = AttemptHistory::new();
        h.record_failure("o/r", 1, 0, 1, &[], "x");
        assert!(h.get("o/r", 2).is_empty());
    }
}