//! Memory lifecycle management for tier promotion, demotion, and expiration
//!
//! Runs on a low-frequency timer (default 1 hour) to manage episode/lesson tiers:
//! - Transient -> Candidate (after enough successes)
//! - Candidate -> Canonical (after enough confirmations)
//! - Transient expiration (after TTL)
//! - Canonical -> Candidate demotion (when evidence contradicts)

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for memory lifecycle management
#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    /// Successes before transient -> candidate
    pub promotion_threshold: u32,
    /// Confirmations before candidate -> canonical
    pub canonical_threshold: u32,
    /// Days before transient data expires
    pub transient_ttl_days: u32,
    /// How often the lifecycle runs
    pub lifecycle_interval: Duration,
}

impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            promotion_threshold: 3,
            canonical_threshold: 10,
            transient_ttl_days: 7,
            lifecycle_interval: Duration::from_hours(1),
        }
    }
}

/// Report from a lifecycle pass
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LifecycleReport {
    /// Episodes/lessons promoted from transient to candidate
    pub promoted: usize,
    /// Candidates distilled to canonical
    pub distilled: usize,
    /// Transient data expired
    pub expired: usize,
    /// Canonical data demoted due to contradicting evidence
    pub demoted: usize,
    /// Whether the pass was skipped due to write contention
    pub skipped_write_contention: bool,
}

impl LifecycleReport {
    pub fn skipped() -> Self {
        Self {
            skipped_write_contention: true,
            ..Default::default()
        }
    }

    /// Whether any changes were made
    pub fn has_changes(&self) -> bool {
        self.promoted > 0 || self.distilled > 0 || self.expired > 0 || self.demoted > 0
    }
}

/// Lifecycle tier transition logic (pure functions, no I/O)
#[derive(Default)]
pub struct LifecycleRules {
    pub config: LifecycleConfig,
}

impl LifecycleRules {
    pub fn new(config: LifecycleConfig) -> Self {
        Self { config }
    }

    /// Should a transient item be promoted to candidate?
    pub fn should_promote(&self, success_count: u32) -> bool {
        success_count >= self.config.promotion_threshold
    }

    /// Should a candidate be promoted to canonical?
    pub fn should_canonicalize(&self, confirmation_count: u32) -> bool {
        confirmation_count >= self.config.canonical_threshold
    }

    /// Should a transient item be expired?
    ///
    /// Human-sourced items never expire — they require explicit removal.
    pub fn should_expire(&self, age_days: u32, is_human_sourced: bool) -> bool {
        if is_human_sourced {
            return false;
        }
        age_days >= self.config.transient_ttl_days
    }

    /// Should a canonical item be demoted due to declining quality?
    ///
    /// Human-sourced items are never automatically demoted. Machine learning
    /// can observe contradictions but only surfaces them for human review.
    pub fn should_demote(&self, recent_failure_rate: f64, is_human_sourced: bool) -> bool {
        if is_human_sourced {
            return false;
        }
        recent_failure_rate > 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lifecycle_rules_promotion() {
        let rules = LifecycleRules::default();
        assert!(!rules.should_promote(2));
        assert!(rules.should_promote(3));
        assert!(rules.should_promote(10));
    }

    #[test]
    fn test_lifecycle_rules_canonicalize() {
        let rules = LifecycleRules::default();
        assert!(!rules.should_canonicalize(9));
        assert!(rules.should_canonicalize(10));
    }

    #[test]
    fn test_lifecycle_rules_expiration() {
        let rules = LifecycleRules::default();
        assert!(!rules.should_expire(6, false));
        assert!(rules.should_expire(7, false));
        assert!(rules.should_expire(30, false));
    }

    #[test]
    fn test_lifecycle_rules_human_never_expires() {
        let rules = LifecycleRules::default();
        assert!(!rules.should_expire(100, true));
        assert!(!rules.should_expire(365, true));
    }

    #[test]
    fn test_lifecycle_rules_demotion() {
        let rules = LifecycleRules::default();
        assert!(!rules.should_demote(0.3, false));
        assert!(!rules.should_demote(0.5, false));
        assert!(rules.should_demote(0.51, false));
    }

    #[test]
    fn test_lifecycle_rules_human_never_demoted() {
        let rules = LifecycleRules::default();
        assert!(!rules.should_demote(0.9, true));
        assert!(!rules.should_demote(1.0, true));
    }

    #[test]
    fn test_lifecycle_report_default() {
        let report = LifecycleReport::default();
        assert!(!report.has_changes());
        assert!(!report.skipped_write_contention);
    }

    #[test]
    fn test_lifecycle_report_skipped() {
        let report = LifecycleReport::skipped();
        assert!(report.skipped_write_contention);
        assert!(!report.has_changes());
    }

    #[test]
    fn test_lifecycle_report_with_changes() {
        let report = LifecycleReport {
            promoted: 2,
            ..Default::default()
        };
        assert!(report.has_changes());
    }

    #[test]
    fn test_custom_config() {
        let config = LifecycleConfig {
            promotion_threshold: 5,
            canonical_threshold: 20,
            transient_ttl_days: 14,
            lifecycle_interval: Duration::from_secs(7200),
        };
        let rules = LifecycleRules::new(config);
        assert!(!rules.should_promote(4));
        assert!(rules.should_promote(5));
        assert!(!rules.should_canonicalize(19));
        assert!(rules.should_canonicalize(20));
        assert!(!rules.should_expire(13, false));
        assert!(rules.should_expire(14, false));
    }
}
