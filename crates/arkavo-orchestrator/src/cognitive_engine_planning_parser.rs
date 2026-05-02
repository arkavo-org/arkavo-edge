//! Plan-response parsers split out of `cognitive_engine_planning`.
//!
//! Two formats are accepted: a structured JSON `JsonExecutionPlan` (for
//! providers with native structured output) and a plain-text "STEP/COMMANDS/
//! VERIFY/CONFIDENCE" block format used as a backwards-compatible fallback.

use crate::cognitive_engine_core::{PlanStep, VerificationCheck};
use crate::cognitive_engine_schema::JsonExecutionPlan;
use crate::error::Result;
use tracing::{debug, warn};

/// Parse a planner response, trying JSON first and falling back to the
/// plain-text format. JSON is preferred because it loses no information;
/// the text path exists for older / non-structured-output providers.
pub fn parse_plan_json_or_text(response: &str) -> Result<Vec<PlanStep>> {
    if let Ok(json_plan) = serde_json::from_str::<JsonExecutionPlan>(response) {
        debug!("Successfully parsed JSON execution plan");
        return convert_json_to_plan_steps(json_plan);
    }
    debug!("JSON parsing failed, falling back to text parser");
    parse_plan_from_response(response)
}

/// Plain-text plan parser. Recognises lines beginning with `STEP `,
/// `COMMANDS:`, `VERIFY:`, and `CONFIDENCE:` and assembles them into
/// successive `PlanStep`s.
pub fn parse_plan_from_response(response: &str) -> Result<Vec<PlanStep>> {
    let mut steps = Vec::new();
    let mut current_step: Option<PlanStep> = None;

    for line in response.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if line.starts_with("STEP ") {
            if let Some(step) = current_step.take() {
                steps.push(step);
            }
            let desc = line
                .split_once(": ")
                .map(|(_, d)| d)
                .unwrap_or(line)
                .to_string();
            current_step = Some(PlanStep {
                step_number: steps.len() + 1,
                description: desc,
                commands: Vec::new(),
                verification: Vec::new(),
                confidence: 0.8,
            });
        } else if line.starts_with("COMMANDS:")
            && let Some(step) = current_step.as_mut()
        {
            step.commands = line
                .strip_prefix("COMMANDS:")
                .unwrap_or("")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if line.starts_with("VERIFY:")
            && let Some(step) = current_step.as_mut()
        {
            step.verification =
                parse_verification_keywords(line.strip_prefix("VERIFY:").unwrap_or(""));
        } else if line.starts_with("CONFIDENCE:")
            && let Some(step) = current_step.as_mut()
            && let Some(conf_str) = line.strip_prefix("CONFIDENCE:")
            && let Ok(conf) = conf_str.trim().parse::<f32>()
        {
            step.confidence = conf;
        }
    }

    if let Some(step) = current_step {
        steps.push(step);
    }

    if steps.is_empty() {
        warn!("No steps parsed from plan, using default");
        steps.push(default_step());
    }

    Ok(steps)
}

fn convert_json_to_plan_steps(json_plan: JsonExecutionPlan) -> Result<Vec<PlanStep>> {
    if json_plan.steps.is_empty() {
        warn!("Empty JSON plan received, using default");
        return Ok(vec![default_step()]);
    }
    let steps = json_plan
        .steps
        .into_iter()
        .map(|js| PlanStep {
            step_number: js.step_number,
            description: js.description,
            commands: js.commands,
            verification: js
                .verification
                .into_iter()
                .filter_map(|v| keyword_to_check(&v))
                .collect(),
            confidence: js.confidence,
        })
        .collect();
    Ok(steps)
}

fn parse_verification_keywords(s: &str) -> Vec<VerificationCheck> {
    s.split(',').filter_map(|w| keyword_to_check(w)).collect()
}

fn keyword_to_check(raw: &str) -> Option<VerificationCheck> {
    let check = raw.trim().to_lowercase();
    if check.contains("test") {
        Some(VerificationCheck::TestsPassing)
    } else if check.contains("lint") {
        Some(VerificationCheck::LinterClean)
    } else if check.contains("build") {
        Some(VerificationCheck::BuildSuccessful)
    } else if check.contains("file_constraint") {
        Some(VerificationCheck::FileConstraint { max_lines: 400 })
    } else {
        None
    }
}

fn default_step() -> PlanStep {
    PlanStep {
        step_number: 1,
        description: "Analyze and fix the issue".to_string(),
        commands: vec!["echo 'Analyzing issue'".to_string()],
        verification: vec![VerificationCheck::BuildSuccessful],
        confidence: 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text_steps() {
        let resp = "\
STEP 1: do the thing
COMMANDS: cmd1, cmd2
VERIFY: tests, build
CONFIDENCE: 0.9
STEP 2: do another
COMMANDS: cmd3
VERIFY: lint
";
        let steps = parse_plan_from_response(resp).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].commands, vec!["cmd1", "cmd2"]);
        assert!((steps[0].confidence - 0.9).abs() < 1e-6);
        assert_eq!(steps[1].commands, vec!["cmd3"]);
    }

    #[test]
    fn empty_text_returns_default_step() {
        let steps = parse_plan_from_response("").unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].step_number, 1);
    }

    #[test]
    fn json_parse_takes_priority() {
        let json = r#"{"steps":[{"step_number":1,"description":"x","commands":["c"],"verification":["tests"],"confidence":0.5}]}"#;
        let steps = parse_plan_json_or_text(json).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].description, "x");
    }

    #[test]
    fn keyword_to_check_recognizes_aliases() {
        assert!(matches!(
            keyword_to_check("tests"),
            Some(VerificationCheck::TestsPassing)
        ));
        assert!(matches!(
            keyword_to_check("LINTER"),
            Some(VerificationCheck::LinterClean)
        ));
        assert!(matches!(
            keyword_to_check("Build"),
            Some(VerificationCheck::BuildSuccessful)
        ));
        assert!(keyword_to_check("nonsense").is_none());
    }
}
