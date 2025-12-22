//! Configuration for the Critic pipeline

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for the Critic verification pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct CriticConfig {
    /// Maximum total time for all checks (ms)
    pub max_total_latency_ms: u64,
    /// Timeout for individual checks (ms)
    pub check_timeout_ms: u64,
    /// Whether to fail fast on first error
    pub fail_fast: bool,
    /// Whether to run checks in parallel where possible
    pub parallel_checks: bool,
    /// Minimum severity to fail the pipeline
    pub min_fail_severity: u8,
    /// Whether semantic checks are enabled
    pub enable_semantic_checks: bool,
    /// Whether HITL checks are enabled
    pub enable_hitl: bool,
}

impl Default for CriticConfig {
    fn default() -> Self {
        Self {
            max_total_latency_ms: 100, // 100ms total budget
            check_timeout_ms: 50,      // 50ms per check
            fail_fast: true,
            parallel_checks: false, // Sequential by default for predictable ordering
            min_fail_severity: 2,   // Warning and above
            enable_semantic_checks: true,
            enable_hitl: false, // Disabled by default
        }
    }
}

impl CriticConfig {
    /// Create a fast config optimized for low latency
    pub fn fast() -> Self {
        Self {
            max_total_latency_ms: 10,
            check_timeout_ms: 5,
            fail_fast: true,
            parallel_checks: true,
            min_fail_severity: 3, // Only errors
            enable_semantic_checks: false,
            enable_hitl: false,
        }
    }

    /// Create a thorough config for maximum quality assurance
    pub fn thorough() -> Self {
        Self {
            max_total_latency_ms: 500,
            check_timeout_ms: 100,
            fail_fast: false,
            parallel_checks: false,
            min_fail_severity: 1, // Info and above
            enable_semantic_checks: true,
            enable_hitl: true,
        }
    }

    /// Get check timeout as Duration
    pub fn check_timeout(&self) -> Duration {
        Duration::from_millis(self.check_timeout_ms)
    }

    /// Get total timeout as Duration
    pub fn total_timeout(&self) -> Duration {
        Duration::from_millis(self.max_total_latency_ms)
    }
}
