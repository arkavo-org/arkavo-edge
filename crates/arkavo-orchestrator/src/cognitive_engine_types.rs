//! Plan and execution result types used across the cognitive engine modules.
//!
//! Kept in their own module so callers (planning, verification, outcome,
//! step) can `use` them without pulling in the full engine.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub id: Uuid,
    pub issue_number: u64,
    pub repository: String,
    pub steps: Vec<PlanStep>,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub step_number: usize,
    pub description: String,
    pub commands: Vec<String>,
    pub verification: Vec<VerificationCheck>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationCheck {
    TestsPassing,
    LinterClean,
    BuildSuccessful,
    FileConstraint { max_lines: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub success: bool,
    pub steps_completed: usize,
    pub total_tokens_used: u32,
    pub verification_results: Vec<VerificationResult>,
    pub final_comment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub check: VerificationCheck,
    pub passed: bool,
    pub details: String,
}
