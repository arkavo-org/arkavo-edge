//! R2: Semantic verifier delta tracking.
//!
//! When the same plan is retried, we want to know whether the verification
//! output is *changing in the right direction* (failures becoming passes)
//! or *thrashing* (new failures appearing that weren't there last time).
//!
//! This module provides a compact digest of a verification run and a
//! comparison function that categorizes the delta between runs:
//!
//! * **Improving** — strictly fewer failures, no new ones.
//! * **Regressing** — new failures that weren't present before.
//! * **Thrashing** — failures changed but total count is similar (probable
//!   flaky test or non-deterministic verifier).
//! * **Stable** — identical set of failures (usually unhelpful: we're stuck).
//! * **Unchanged** — verification digest is byte-identical (strongest
//!   signal that we need a different approach).
//!
//! Digests are intentionally deterministic so they can be persisted /
//! compared across processes later (C1 cross-process learning).

use crate::cognitive_engine_core::{VerificationCheck, VerificationResult};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Compact, deterministic digest of one verification run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifierSnapshot {
    /// Sorted, stringified check kinds that failed.
    pub failing_checks: BTreeSet<String>,
    /// Total number of checks run.
    pub total_checks: usize,
    /// Number of passing checks.
    pub passing: usize,
    /// Stable 64-bit digest of the failure fingerprint.
    pub digest: u64,
}

impl VerifierSnapshot {
    pub fn from_results(results: &[VerificationResult]) -> Self {
        let mut failing: BTreeSet<String> = BTreeSet::new();
        let mut passing = 0usize;
        for r in results {
            if r.passed {
                passing += 1;
            } else {
                failing.insert(format_check(&r.check));
            }
        }
        let digest = compute_digest(&failing);
        Self {
            failing_checks: failing,
            total_checks: results.len(),
            passing,
            digest,
        }
    }

    /// Number of failing checks.
    pub fn failing_count(&self) -> usize {
        self.failing_checks.len()
    }
}

/// Classification of the delta between two verifier snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaKind {
    /// Digest is byte-identical.
    Unchanged,
    /// Same set of failures, different count.
    Stable,
    /// Strictly fewer failures; all remaining failures were in `before`.
    Improving,
    /// New failures appeared that weren't in `before`.
    Regressing,
    /// Failures rotated (some fixed, some new), net ~= 0. Probable flaky.
    Thrashing,
}

impl DeltaKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Stable => "stable",
            Self::Improving => "improving",
            Self::Regressing => "regressing",
            Self::Thrashing => "thrashing",
        }
    }

    /// Is this delta a useful forward-progress signal?
    pub fn is_progress(&self) -> bool {
        matches!(self, Self::Improving)
    }

    /// Should the caller consider giving up this retry loop?
    pub fn suggests_giveup(&self) -> bool {
        matches!(self, Self::Unchanged | Self::Stable | Self::Thrashing)
    }
}

#[derive(Debug, Clone)]
pub struct VerifierDelta {
    pub kind: DeltaKind,
    pub newly_failing: Vec<String>,
    pub newly_passing: Vec<String>,
    pub before_failing_count: usize,
    pub after_failing_count: usize,
}

impl VerifierDelta {
    pub fn compute(before: &VerifierSnapshot, after: &VerifierSnapshot) -> Self {
        if before.digest == after.digest {
            return Self {
                kind: DeltaKind::Unchanged,
                newly_failing: Vec::new(),
                newly_passing: Vec::new(),
                before_failing_count: before.failing_count(),
                after_failing_count: after.failing_count(),
            };
        }

        let newly_failing: Vec<String> = after
            .failing_checks
            .difference(&before.failing_checks)
            .cloned()
            .collect();
        let newly_passing: Vec<String> = before
            .failing_checks
            .difference(&after.failing_checks)
            .cloned()
            .collect();

        let kind = classify(
            before.failing_count(),
            after.failing_count(),
            newly_failing.len(),
            newly_passing.len(),
        );

        Self {
            kind,
            newly_failing,
            newly_passing,
            before_failing_count: before.failing_count(),
            after_failing_count: after.failing_count(),
        }
    }

    /// Render a compact one-line summary suitable for progress updates.
    pub fn summary(&self) -> String {
        format!(
            "verifier {} ({} -> {} failing; +{} new, -{} fixed)",
            self.kind.label(),
            self.before_failing_count,
            self.after_failing_count,
            self.newly_failing.len(),
            self.newly_passing.len()
        )
    }
}

fn classify(
    before_fail: usize,
    after_fail: usize,
    newly_failing: usize,
    newly_passing: usize,
) -> DeltaKind {
    if newly_failing == 0 && newly_passing == 0 {
        // Same set; digest would have been equal, but maybe total counts
        // differ. Treat as Stable.
        return DeltaKind::Stable;
    }
    if newly_failing == 0 && newly_passing > 0 && after_fail < before_fail {
        return DeltaKind::Improving;
    }
    if newly_failing > 0 && newly_passing > 0 {
        // Failures rotated. If the net change is small (within 1), call it
        // thrashing; otherwise regressing / improving based on direction.
        let net = after_fail as i64 - before_fail as i64;
        if net.abs() <= 1 {
            return DeltaKind::Thrashing;
        }
        if net < 0 {
            return DeltaKind::Improving;
        }
        return DeltaKind::Regressing;
    }
    if newly_failing > 0 {
        return DeltaKind::Regressing;
    }
    DeltaKind::Stable
}

fn format_check(check: &VerificationCheck) -> String {
    // Deterministic short form kept intentionally terse; future variants
    // should map to short stable labels here.
    match check {
        VerificationCheck::TestsPassing => "tests".to_string(),
        VerificationCheck::LinterClean => "linter".to_string(),
        VerificationCheck::BuildSuccessful => "build".to_string(),
        VerificationCheck::FileConstraint { max_lines } => {
            format!("file-constraint:{max_lines}")
        }
    }
}

/// Deterministic 64-bit digest of the failing-checks set. Uses FNV-1a,
/// which is portable and has no external dependencies.
fn compute_digest(failing: &BTreeSet<String>) -> u64 {
    // FNV-1a 64-bit: offset basis and prime, per the reference spec.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x100_0000_01b3;
    for s in failing {
        for b in s.as_bytes() {
            hash ^= u64::from(*b);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        // Separator so "ab" + "c" differs from "a" + "bc".
        hash ^= 0xff;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(check: VerificationCheck, passed: bool) -> VerificationResult {
        VerificationResult {
            check,
            passed,
            details: String::new(),
        }
    }

    #[test]
    fn snapshot_counts_and_digest() {
        let snap = VerifierSnapshot::from_results(&[
            res(VerificationCheck::TestsPassing, true),
            res(VerificationCheck::LinterClean, false),
            res(VerificationCheck::BuildSuccessful, false),
        ]);
        assert_eq!(snap.total_checks, 3);
        assert_eq!(snap.passing, 1);
        assert_eq!(snap.failing_count(), 2);
        assert!(snap.failing_checks.contains("linter"));
        assert!(snap.failing_checks.contains("build"));
    }

    #[test]
    fn identical_digests_for_same_failures() {
        let a = VerifierSnapshot::from_results(&[
            res(VerificationCheck::LinterClean, false),
            res(VerificationCheck::BuildSuccessful, false),
        ]);
        let b = VerifierSnapshot::from_results(&[
            res(VerificationCheck::BuildSuccessful, false),
            res(VerificationCheck::LinterClean, false),
        ]);
        assert_eq!(a.digest, b.digest);
    }

    #[test]
    fn improving_detected() {
        let before = VerifierSnapshot::from_results(&[
            res(VerificationCheck::LinterClean, false),
            res(VerificationCheck::BuildSuccessful, false),
        ]);
        let after =
            VerifierSnapshot::from_results(&[res(VerificationCheck::BuildSuccessful, false)]);
        let delta = VerifierDelta::compute(&before, &after);
        assert_eq!(delta.kind, DeltaKind::Improving);
        assert!(delta.kind.is_progress());
        assert_eq!(delta.newly_passing, vec!["linter".to_string()]);
    }

    #[test]
    fn regressing_detected() {
        let before = VerifierSnapshot::from_results(&[res(VerificationCheck::LinterClean, false)]);
        let after = VerifierSnapshot::from_results(&[
            res(VerificationCheck::LinterClean, false),
            res(VerificationCheck::BuildSuccessful, false),
        ]);
        let delta = VerifierDelta::compute(&before, &after);
        assert_eq!(delta.kind, DeltaKind::Regressing);
        assert!(!delta.kind.is_progress());
        assert_eq!(delta.newly_failing, vec!["build".to_string()]);
    }

    #[test]
    fn thrashing_detected_when_failures_rotate() {
        let before = VerifierSnapshot::from_results(&[res(VerificationCheck::LinterClean, false)]);
        let after =
            VerifierSnapshot::from_results(&[res(VerificationCheck::BuildSuccessful, false)]);
        let delta = VerifierDelta::compute(&before, &after);
        assert_eq!(delta.kind, DeltaKind::Thrashing);
        assert!(delta.kind.suggests_giveup());
    }

    #[test]
    fn unchanged_when_identical() {
        let a = VerifierSnapshot::from_results(&[res(VerificationCheck::LinterClean, false)]);
        let b = VerifierSnapshot::from_results(&[res(VerificationCheck::LinterClean, false)]);
        let delta = VerifierDelta::compute(&a, &b);
        assert_eq!(delta.kind, DeltaKind::Unchanged);
        assert!(delta.kind.suggests_giveup());
    }

    #[test]
    fn summary_includes_counts() {
        let before = VerifierSnapshot::from_results(&[
            res(VerificationCheck::LinterClean, false),
            res(VerificationCheck::BuildSuccessful, false),
        ]);
        let after =
            VerifierSnapshot::from_results(&[res(VerificationCheck::BuildSuccessful, false)]);
        let delta = VerifierDelta::compute(&before, &after);
        let s = delta.summary();
        assert!(s.contains("improving"));
        assert!(s.contains("2 -> 1"));
    }
}
