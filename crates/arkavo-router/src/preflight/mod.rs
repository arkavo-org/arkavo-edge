//! Pre-flight moderation for blocking requests before LLM inference
//!
//! This module provides fast-path policy evaluation using TØR-G boolean circuits.
//! Requests are checked against registered policies before any LLM inference occurs,
//! blocking policy-violating requests with sub-microsecond latency.
//!
//! # Example
//!
//! ```ignore
//! use arkavo_router::preflight::{PreflightModerator, PolicyId, PreflightFeature};
//!
//! let moderator = PreflightModerator::new();
//!
//! // Register a policy that blocks PII
//! moderator.register_graph(
//!     PolicyId::new("block_pii"),
//!     not_circuit,  // NOT(input) - blocks when true
//!     vec![PreflightFeature::InputContainsPII],
//! );
//!
//! // Check a request
//! match moderator.check("My SSN is 123-45-6789") {
//!     ModerationResult::Allow => { /* proceed with LLM */ }
//!     ModerationResult::Block { policy_id, reason, .. } => {
//!         // Request blocked
//!     }
//! }
//! ```

mod circuit;
mod features;
mod moderator;
mod result;

pub use features::PreflightFeature;
pub use moderator::{PolicyId, PreflightModerator};
pub use result::ModerationResult;
