//! Final-outcome bookkeeping for the cognitive engine's `execute` loop.
//!
//! Centralizes the bits of state that are easy to desynchronize when the
//! main loop has multiple early-exit paths: the abort cause (FailureKind +
//! detail string), the final user-facing comment, and the attempt-history
//! recording. Keeping this in one module shrinks `cognitive_engine_core`
//! and makes the success/failure invariant easier to test in isolation.

use crate::attempt_history::{AttemptHistory, FailureKind};
use crate::cognitive_engine_types::{ExecutionPlan, VerificationResult};
use std::sync::Arc;

/// Cumulative state captured during a single `execute` run; used to make
/// the post-loop bookkeeping (final comment, attempt-history recording)
/// a single function rather than inlined into the orchestrator.
pub struct ExecutionOutcome {
    pub steps_completed: usize,
    pub total_tokens: u32,
    pub verification_results: Vec<VerificationResult>,
    pub abort_kind: Option<FailureKind>,
    pub abort_detail: Option<String>,
}

impl ExecutionOutcome {
    pub fn new() -> Self {
        Self {
            steps_completed: 0,
            total_tokens: 0,
            verification_results: Vec::new(),
            abort_kind: None,
            abort_detail: None,
        }
    }

    pub fn is_success(&self, plan_step_count: usize) -> bool {
        self.steps_completed == plan_step_count
            && self.verification_results.iter().all(|r| r.passed)
    }

    /// Build the user-facing comment posted at the end of the run.
    pub fn final_comment(&self, plan_step_count: usize, budget: u32) -> String {
        if self.is_success(plan_step_count) {
            format!(
                "✅ Issue completed successfully!\n\n\
                - Steps completed: {}/{}\n\
                - Tokens used: {}/{}\n\
                - All verifications passed",
                self.steps_completed, plan_step_count, self.total_tokens, budget
            )
        } else {
            format!(
                "⚠️ Issue execution incomplete\n\n\
                - Steps completed: {}/{}\n\
                - Tokens used: {}/{}\n\
                - Verification failures: {}",
                self.steps_completed,
                plan_step_count,
                self.total_tokens,
                budget,
                self.verification_results
                    .iter()
                    .filter(|r| !r.passed)
                    .count()
            )
        }
    }

    /// Record the run outcome into the shared attempt history. On success,
    /// clear the history so a future re-run starts fresh; on failure, pick
    /// the most-specific FailureKind we observed and record it.
    pub fn record_into_history(
        &self,
        history: &Arc<AttemptHistory>,
        repository: &str,
        issue_number: u64,
        plan_step_count: usize,
    ) {
        if self.is_success(plan_step_count) {
            history.clear(repository, issue_number);
            return;
        }
        let verification_fail_count = self
            .verification_results
            .iter()
            .filter(|r| !r.passed)
            .count();
        let default_kind = if verification_fail_count > 0 {
            FailureKind::VerificationFailed
        } else {
            FailureKind::Other
        };
        let kind = self.abort_kind.clone().unwrap_or(default_kind);
        let summary = self.abort_detail.clone().unwrap_or_else(|| {
            format!(
                "execution incomplete: {}/{} steps, {} verification failure(s)",
                self.steps_completed, plan_step_count, verification_fail_count
            )
        });
        history.record_failure_kind(
            repository,
            issue_number,
            kind,
            self.steps_completed,
            plan_step_count,
            &self.verification_results,
            summary,
        );
    }
}

/// Record the special-case failure when plan validation rejects a plan
/// before any step has run. Kept here so the recording path is identical
/// in shape to the post-loop one.
pub fn record_plan_invalid(history: &Arc<AttemptHistory>, plan: &ExecutionPlan, summary: &str) {
    history.record_failure_kind(
        &plan.repository,
        plan.issue_number,
        FailureKind::PlanInvalid,
        0,
        plan.steps.len(),
        &[],
        format!("plan validation failed: {summary}"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cognitive_engine_types::VerificationCheck;

    fn passed() -> VerificationResult {
        VerificationResult {
            check: VerificationCheck::TestsPassing,
            passed: true,
            details: String::new(),
        }
    }

    fn failed() -> VerificationResult {
        VerificationResult {
            check: VerificationCheck::TestsPassing,
            passed: false,
            details: "boom".into(),
        }
    }

    #[test]
    fn success_when_all_steps_done_and_verified() {
        let mut o = ExecutionOutcome::new();
        o.steps_completed = 3;
        o.verification_results = vec![passed(), passed()];
        assert!(o.is_success(3));
    }

    #[test]
    fn failure_when_verification_fails() {
        let mut o = ExecutionOutcome::new();
        o.steps_completed = 3;
        o.verification_results = vec![passed(), failed()];
        assert!(!o.is_success(3));
    }

    #[test]
    fn failure_when_steps_incomplete() {
        let mut o = ExecutionOutcome::new();
        o.steps_completed = 2;
        o.verification_results = vec![passed()];
        assert!(!o.is_success(3));
    }

    #[test]
    fn final_comment_success_text() {
        let mut o = ExecutionOutcome::new();
        o.steps_completed = 2;
        o.total_tokens = 100;
        o.verification_results = vec![passed()];
        let c = o.final_comment(2, 1000);
        assert!(c.contains("completed successfully"));
        assert!(c.contains("100/1000"));
    }

    #[test]
    fn final_comment_failure_includes_count() {
        let mut o = ExecutionOutcome::new();
        o.steps_completed = 1;
        o.total_tokens = 50;
        o.verification_results = vec![failed(), failed(), passed()];
        let c = o.final_comment(3, 1000);
        assert!(c.contains("incomplete"));
        assert!(c.contains("Verification failures: 2"));
    }
}
