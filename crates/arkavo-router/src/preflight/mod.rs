//! Pre-flight moderation for blocking requests before LLM inference
//!
//! This module provides fast-path policy evaluation using TØR-G boolean circuits.
//! Requests are checked against registered policies before any LLM inference occurs,
//! blocking policy-violating requests with sub-microsecond latency.
//!
//! # Runtime Policy Configuration
//!
//! Policies are loaded from user configuration at runtime, not hardcoded.
//! Configuration file: `~/.config/arkavo/policies.toml`
//!
//! ```toml
//! [[policies]]
//! id = "block_pii"
//! features = ["InputContainsPII"]
//! action = "block"
//!
//! [[policies]]
//! id = "block_sql_injection"
//! features = ["InputContainsSQLKeywords"]
//! action = "block"
//! ```
//!
//! # Example
//!
//! ```ignore
//! use arkavo_router::preflight::{PreflightModerator, load_policies_from_config};
//!
//! // Load policies from user configuration
//! let moderator = load_policies_from_config()?;
//!
//! // Check a request
//! match moderator.check("user input") {
//!     ModerationResult::Allow => { /* proceed with LLM */ }
//!     ModerationResult::Block { policy_id, reason, .. } => {
//!         // Request blocked by user-configured policy
//!     }
//! }
//! ```

mod circuit;
mod config;
mod features;
mod moderator;
mod result;

pub use config::{
    load_policies_from_agents_md, load_policies_from_config, AgentConfig, PolicyAction,
    PolicyConfig, PolicyFileConfig, PreflightConfig,
};
pub use features::PreflightFeature;
pub use moderator::{PolicyId, PreflightModerator};
pub use result::ModerationResult;
