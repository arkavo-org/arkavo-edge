//! Shadow consolidation with Fable — outside the runtime.
//!
//! A standalone, audit-plane job that takes existing episode traces (with
//! observe calls stripped, per the lesson-synthesis rule), batches them to
//! Fable, and writes three artifacts to disk:
//!
//! - `lessons.json` — action-named lessons ([`proposal::ActionLesson`]).
//! - `proposals.json` — candidate tightenings, the draft `proposal.rs` JSON shape ([`proposal::Proposal`]).
//! - `cost_ledger.json` — real token/dollar accounting plus a recommended value for `budget.per_layer["consolidation"]` ([`proposal::CostLedger`]).
//!
//! It also emits `findings.json` — [`proposal::FindingCard`] seed cards
//! projected from the genuine lessons and proposals, so the findings-inbox UI
//! can be populated from real output instead of invented fixtures.
//!
//! The job has **zero runtime contact**: it depends on no arkavo runtime
//! crate, opens the episode store read-only, and reaches only Fable over the
//! network.

pub mod episodes;
pub mod fable;
pub mod proposal;
pub mod run;
pub mod synthesis;

pub use run::{Config, RunSummary, execute};
