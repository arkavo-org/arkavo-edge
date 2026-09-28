//! The DLP sentinel: the classification cascade, the streaming holdback that
//! gives it something to inspect before release, and the executor that keeps it
//! off the per-call path.
//!
//! The design in one line: **the sentinel labels, the policy decision point
//! authorizes**. Nothing in this crate returns an allow or a block. Every tier
//! returns evidence — what it saw, how sure it is, which version of which
//! detector said so — and something else decides what that means. A classifier
//! that could also authorize would be a second policy engine, disagreeing with
//! the first at exactly the moments that matter.

pub mod budget;
pub mod calibration;
pub mod cascade;
pub mod holdback;
#[cfg(feature = "protected-weights")]
pub mod loader;
pub mod merge;
pub mod pattern_tier;
pub mod scoring;

pub use budget::{ANONYMOUS, ProbeBudget, ProbeVerdict};
pub use calibration::{CalibrationTable, ThresholdSource};
pub use cascade::{CASCADE_BUDGET, Cascade, CascadeTier};
pub use holdback::{DEFAULT_OVERLAP_BYTES, DEFAULT_WINDOW_BYTES, Holdback, HoldbackState, Window};
#[cfg(feature = "protected-weights")]
pub use loader::{LoadError, open_sentinel};
pub use merge::{INFERRED_SOURCE, merge_evidence};
pub use pattern_tier::PatternTier;
pub use scoring::{
    RawLabel, SENTINEL_TIER_NAME, ScoringError, ScoringExecutor, ScoringModel, SentinelTier,
};
