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
//! can be populated from real output instead of invented fixtures — plus
//! `rejects.json` ([`validate::Reject`]) recording every contract violation
//! with a reason (the source of the ledger's contract-conformance rate), and
//! a `raw/` archive of each verbatim Fable reply for contract-evolution
//! regression.
//!
//! The job has **zero runtime contact**: it depends on no arkavo runtime
//! crate (its only arkavo dependency is the std-only lesson-synthesis rules
//! crate shared with the consolidation teacher), opens the episode store
//! read-only, and reaches only Fable over the network — under a hard
//! run-level cost ceiling (`--max-run-cost-usd`) that stops the run between
//! batches, keeps partial outputs, and marks the ledger budget-exhausted.
//!
//! This crate is transitional: it exists to validate the consolidation
//! contract before the runtime teacher takes over, and it folds into the
//! runtime as a shadow mode once that validation holds. The artifacts are
//! the durable contract — build consumers against the JSON shapes, not
//! against this binary.

pub mod episodes;
pub mod fable;
pub mod proposal;
pub mod run;
pub mod synthesis;
pub mod validate;

pub use run::{Config, RunSummary, execute};
