//! Arkavo Critic - Verification pipeline for LLM response quality assurance
//!
//! The Critic provides a pluggable verification pipeline that validates
//! LLM responses before they're considered complete. Checks are run in
//! priority order, with fast checks first.
//!
//! # Check Types
//!
//! | Check | Priority | Purpose | Latency Target |
//! |-------|----------|---------|----------------|
//! | CircuitCheck | 5 | TØR-G boolean circuit policies | <1μs |
//! | SchemaCheck | 10 | Validate tool call schemas | <1ms |
//! | PolicyCheck | 15 | SBE invariant enforcement | <1ms |
//! | LintCheck | 20 | Code/response quality | <2ms |
//! | SemanticCheck | 100 | LLM coherence (local model) | <50ms |
//!
//! # Example
//!
//! ```ignore
//! use arkavo_critic::{CriticPipeline, CircuitCheck, SchemaCheck, SemanticCheck};
//!
//! let pipeline = CriticPipeline::new()
//!     .add_check(CircuitCheck::new())
//!     .add_check(SchemaCheck::new())
//!     .add_check(SemanticCheck::new());
//!
//! let result = pipeline.verify(&input).await;
//! if result.passed {
//!     // Response is verified
//! }
//! ```

mod checks;
mod config;
mod error;
mod evidence;
pub mod features;
mod judge;
mod pipeline;
pub mod response_analyzer;
mod validator;

pub use checks::{
    CheckResult, CircuitCheck, CodeVerificationCheck, LintCheck, PolicyCheck, PolicyId,
    SchemaCheck, SemanticCheck, VerificationCheck, VerificationInput,
};
pub use config::CriticConfig;
pub use error::{Error, Result};
pub use evidence::{CheckSeverity, VerificationEvidence, VerificationStatus};
pub use features::FeatureId;
pub use judge::{IssueType, JudgmentResult, ResponseJudge};
pub use pipeline::{CriticPipeline, PipelineResult};
pub use response_analyzer::{AnalysisResult, DetectedIssue, ResponseAnalyzer};
pub use validator::{ResponseValidator, ValidationError};

/// Create a CriticPipeline with default checks
///
/// Includes:
/// - CircuitCheck (priority 5): TØR-G boolean circuit policies
/// - SchemaCheck (priority 10): Tool call schema validation
/// - PolicyCheck (priority 15): Security policy enforcement
pub fn default_pipeline() -> CriticPipeline {
    CriticPipeline::new()
        .add_check(CircuitCheck::new())
        .add_check(SchemaCheck::new())
        .add_check(PolicyCheck::with_security_defaults())
}
