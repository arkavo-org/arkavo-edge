//! Pre-flight moderation for blocking requests before LLM inference
//!
//! This module provides fast-path policy evaluation using TØR-G boolean circuits.
//! Requests are checked against registered policies before any LLM inference occurs,
//! blocking policy-violating requests with sub-microsecond latency.
//!
//! # Runtime Policy Configuration
//!
//! Policies are defined per-agent in AGENTS.md frontmatter, not hardcoded.
//! The loader searches for AGENTS.md in the current directory and parent directories.
//!
//! ```yaml
//! ---
//! name: secure-agent
//! preflight:
//!   policies:
//!     - id: block_pii
//!       features: [InputContainsPII]
//!       action: block
//!     - id: block_sql_injection
//!       features: [InputContainsSQLKeywords]
//!       action: block
//! ---
//! ```
//!
//! # Example
//!
//! ```ignore
//! use arkavo_router::preflight::{PreflightModerator, load_policies_from_config};
//!
//! // Load policies from AGENTS.md in current/parent directories
//! let moderator = load_policies_from_config()?;
//!
//! // Check a request
//! match moderator.check("user input") {
//!     ModerationResult::Allow => { /* proceed with LLM */ }
//!     ModerationResult::Block { policy_id, reason, .. } => {
//!         // Request blocked by agent-configured policy
//!     }
//! }
//! ```

mod circuit;
mod config;
mod features;
mod moderator;
mod result;

pub use config::{
    AgentConfig, BudgetYamlConfig, KasYamlConfig, PolicyAction, PolicyConfig, PolicyFileConfig,
    PreflightConfig, build_moderator_from_config, load_agent_config,
    load_agent_config_from_agents_md, load_policies_from_agents_md, load_policies_from_config,
};
pub use features::PreflightFeature;
pub use moderator::{PolicyId, PreflightModerator};
pub use result::ModerationResult;
