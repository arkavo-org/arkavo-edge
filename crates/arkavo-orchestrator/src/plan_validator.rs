//! Plan contract validation (pre-flight check).
//!
//! Catches malformed plans before execution wastes LLM/shell calls on them.
//! Runs fast, synchronous structural checks after planning and before the
//! step execution loop starts.

use crate::cognitive_engine_core::{ExecutionPlan, PlanStep, VerificationCheck};

/// A contract violation found during pre-flight validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractViolation {
    pub step_number: Option<usize>,
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for ContractViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.step_number {
            Some(n) => write!(f, "[step {n}] {}: {}", self.code, self.message),
            None => write!(f, "{}: {}", self.code, self.message),
        }
    }
}

/// Result of validating an execution plan.
#[derive(Debug, Clone, Default)]
pub struct ValidationReport {
    pub violations: Vec<ContractViolation>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.violations.is_empty()
    }

    pub fn summary(&self) -> String {
        if self.is_valid() {
            "valid".to_string()
        } else {
            self.violations
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        }
    }
}

/// Validate the structural integrity of an execution plan.
///
/// Checks performed:
/// - The plan has at least one step.
/// - Step numbers are strictly increasing and 1-based contiguous.
/// - Every step has a non-empty description.
/// - Every step has at least one command.
/// - No command exceeds a sane length (guards against accidental prompt
///   injection through runaway LLM output).
/// - `confidence` is in `[0.0, 1.0]`.
/// - Mutating-looking steps declare at least one verification check.
pub fn validate(plan: &ExecutionPlan) -> ValidationReport {
    let mut violations = Vec::new();

    if plan.steps.is_empty() {
        violations.push(ContractViolation {
            step_number: None,
            code: "EMPTY_PLAN",
            message: "plan contains no steps".to_string(),
        });
        return ValidationReport { violations };
    }

    for (idx, step) in plan.steps.iter().enumerate() {
        let expected = idx + 1;
        if step.step_number != expected {
            violations.push(ContractViolation {
                step_number: Some(step.step_number),
                code: "STEP_NUMBERING",
                message: format!(
                    "expected step_number={expected} at position {idx}, got {}",
                    step.step_number
                ),
            });
        }

        if step.description.trim().is_empty() {
            violations.push(ContractViolation {
                step_number: Some(step.step_number),
                code: "EMPTY_DESCRIPTION",
                message: "description is empty".to_string(),
            });
        }

        if step.commands.is_empty() {
            violations.push(ContractViolation {
                step_number: Some(step.step_number),
                code: "NO_COMMANDS",
                message: "step has no commands".to_string(),
            });
        }

        for cmd in &step.commands {
            if cmd.trim().is_empty() {
                violations.push(ContractViolation {
                    step_number: Some(step.step_number),
                    code: "EMPTY_COMMAND",
                    message: "command string is empty".to_string(),
                });
            }
            if cmd.len() > MAX_COMMAND_LEN {
                violations.push(ContractViolation {
                    step_number: Some(step.step_number),
                    code: "COMMAND_TOO_LONG",
                    message: format!("command length {} exceeds {MAX_COMMAND_LEN}", cmd.len()),
                });
            }
        }

        if !(0.0..=1.0).contains(&step.confidence) {
            violations.push(ContractViolation {
                step_number: Some(step.step_number),
                code: "INVALID_CONFIDENCE",
                message: format!("confidence={} is outside [0.0, 1.0]", step.confidence),
            });
        }

        if looks_mutating(step) && step.verification.is_empty() {
            violations.push(ContractViolation {
                step_number: Some(step.step_number),
                code: "UNVERIFIED_MUTATION",
                message: "mutating step declares no verification checks".to_string(),
            });
        }

        // Require at least one recognizable verification kind for steps
        // that do declare verification (no all-custom-string noise).
        if !step.verification.is_empty()
            && !step.verification.iter().any(|v| {
                matches!(
                    v,
                    VerificationCheck::TestsPassing
                        | VerificationCheck::LinterClean
                        | VerificationCheck::BuildSuccessful
                        | VerificationCheck::FileConstraint { .. }
                )
            })
        {
            violations.push(ContractViolation {
                step_number: Some(step.step_number),
                code: "NO_KNOWN_VERIFICATION",
                message: "no recognized verification check present".to_string(),
            });
        }
    }

    ValidationReport { violations }
}

const MAX_COMMAND_LEN: usize = 4096;

/// Heuristic: does this step look like it writes to disk/git?
fn looks_mutating(step: &PlanStep) -> bool {
    const MUTATING: &[&str] = &[
        "write",
        "edit",
        "create",
        "delete",
        "remove",
        "modify",
        "update",
        "refactor",
        "rename",
        "move",
        "patch",
        "git commit",
        "git add",
        "git push",
        "git merge",
        "git rebase",
        "cargo fmt",
        "apply",
        "fix",
    ];
    let desc = step.description.to_lowercase();
    if MUTATING.iter().any(|kw| desc.contains(kw)) {
        return true;
    }
    step.commands
        .iter()
        .any(|c| MUTATING.iter().any(|kw| c.to_lowercase().contains(kw)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn make_plan(steps: Vec<PlanStep>) -> ExecutionPlan {
        ExecutionPlan {
            id: Uuid::new_v4(),
            issue_number: 1,
            repository: "owner/repo".to_string(),
            steps,
            estimated_tokens: 1000,
        }
    }

    fn ok_step(n: usize) -> PlanStep {
        PlanStep {
            step_number: n,
            description: "read the file".to_string(),
            commands: vec!["cat src/lib.rs".to_string()],
            verification: vec![VerificationCheck::BuildSuccessful],
            confidence: 0.9,
        }
    }

    #[test]
    fn test_valid_plan_passes() {
        let plan = make_plan(vec![ok_step(1), ok_step(2)]);
        let report = validate(&plan);
        assert!(report.is_valid(), "got: {}", report.summary());
    }

    #[test]
    fn test_empty_plan_rejected() {
        let plan = make_plan(vec![]);
        let report = validate(&plan);
        assert!(!report.is_valid());
        assert_eq!(report.violations[0].code, "EMPTY_PLAN");
    }

    #[test]
    fn test_bad_numbering_detected() {
        let mut s = ok_step(1);
        s.step_number = 5;
        let plan = make_plan(vec![s]);
        let report = validate(&plan);
        assert!(report.violations.iter().any(|v| v.code == "STEP_NUMBERING"));
    }

    #[test]
    fn test_empty_commands_detected() {
        let mut s = ok_step(1);
        s.commands = vec![];
        let plan = make_plan(vec![s]);
        let report = validate(&plan);
        assert!(report.violations.iter().any(|v| v.code == "NO_COMMANDS"));
    }

    #[test]
    fn test_empty_command_string_detected() {
        let mut s = ok_step(1);
        s.commands = vec!["   ".to_string()];
        let plan = make_plan(vec![s]);
        let report = validate(&plan);
        assert!(report.violations.iter().any(|v| v.code == "EMPTY_COMMAND"));
    }

    #[test]
    fn test_out_of_range_confidence_detected() {
        let mut s = ok_step(1);
        s.confidence = 1.5;
        let plan = make_plan(vec![s]);
        let report = validate(&plan);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.code == "INVALID_CONFIDENCE")
        );
    }

    #[test]
    fn test_mutating_without_verification_detected() {
        let s = PlanStep {
            step_number: 1,
            description: "edit the main function".to_string(),
            commands: vec!["sed -i s/foo/bar/ src/main.rs".to_string()],
            verification: vec![],
            confidence: 0.8,
        };
        let plan = make_plan(vec![s]);
        let report = validate(&plan);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.code == "UNVERIFIED_MUTATION")
        );
    }

    #[test]
    fn test_non_mutating_no_verification_ok() {
        let s = PlanStep {
            step_number: 1,
            description: "read and analyze file".to_string(),
            commands: vec!["cat Cargo.toml".to_string()],
            verification: vec![],
            confidence: 0.9,
        };
        let plan = make_plan(vec![s]);
        let report = validate(&plan);
        assert!(report.is_valid(), "got: {}", report.summary());
    }

    #[test]
    fn test_command_too_long_detected() {
        let mut s = ok_step(1);
        s.commands = vec!["x".repeat(MAX_COMMAND_LEN + 1)];
        let plan = make_plan(vec![s]);
        let report = validate(&plan);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.code == "COMMAND_TOO_LONG")
        );
    }
}
