//! Arkavo Critic - Verification pipeline for LLM response quality assurance
//!
//! The Critic provides a pluggable verification pipeline that validates
//! LLM responses before they're considered complete. Checks are run in
//! priority order, with fast checks first.
//!
//! # Check Types
//!
//! | Check | Purpose | Latency Target |
//! |-------|---------|----------------|
//! | SchemaCheck | Validate tool call schemas | <1ms |
//! | LintCheck | Code/response quality | <2ms |
//! | PolicyCheck | SBE invariant enforcement | <1ms |
//! | SemanticCheck | LLM coherence (local model) | <50ms |
//! | HumanApproval | HITL verification | async |
//!
//! # Example
//!
//! ```ignore
//! use arkavo_critic::{CriticPipeline, SchemaCheck, SemanticCheck};
//!
//! let pipeline = CriticPipeline::new()
//!     .add_check(SchemaCheck::new(&tools))
//!     .add_check(SemanticCheck::new().await?);
//!
//! let result = pipeline.verify(&input).await?;
//! if result.passed() {
//!     // Response is verified
//! }
//! ```

mod checks;
mod config;
mod error;
mod evidence;
mod pipeline;

pub use checks::{
    CheckResult, LintCheck, PolicyCheck, SchemaCheck, SemanticCheck, VerificationCheck,
    VerificationInput,
};
pub use config::CriticConfig;
pub use error::{Error, Result};
pub use evidence::{CheckSeverity, VerificationEvidence, VerificationStatus};
pub use pipeline::{CriticPipeline, PipelineResult};
