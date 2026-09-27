//! Verification checks for the Critic pipeline

mod circuit;
mod code;
mod lint;
mod policy;
mod rubric;
mod schema;
mod semantic;
#[cfg(feature = "sentinel")]
mod sentinel;
mod traits;

pub use circuit::{CircuitCheck, PolicyId};
pub use code::CodeVerificationCheck;
pub use lint::LintCheck;
pub use policy::PolicyCheck;
pub use rubric::RubricCheck;
pub use schema::SchemaCheck;
pub use semantic::SemanticCheck;
#[cfg(feature = "sentinel")]
pub use sentinel::{ClassificationSource, SENTINEL_PRIORITY, SentinelCheck, SentinelEvidence};
pub use traits::{CheckResult, VerificationCheck, VerificationInput};
