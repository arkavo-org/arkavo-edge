//! Turn a per-category batch of action→outcome pairs into a Fable prompt and
//! parse Fable's reply into the output contract (lessons + tightenings).
//!
//! The prompt enforces the lesson-synthesis rule at the instruction level: the
//! evidence already has observe/list calls stripped, and the model is told not
//! to reason about tool sequencing. Compliance is **not trusted** — every item
//! that fails to parse becomes a [`Reject`] record rather than a silent drop,
//! so non-conformance is counted, not hidden.

use serde_json::{Value, json};

use crate::episodes::CategoryBatch;
use crate::proposal::{ActionLesson, Proposal, Severity};
use crate::validate::{Reject, RejectKind};

/// System prompt: defines Fable's role as the offline consolidation teacher
/// and pins the exact JSON output contract.
pub const SYSTEM_PROMPT: &str = "\
You are the offline consolidation teacher for an agent swarm. You are given \
action->outcome evidence from many episodes in one task category, with \
read-only observe/list calls already removed. Reason about WHICH ACTION \
produces good outcomes under WHICH CONDITION. Do not discuss observe, list, \
or tool sequencing — the agent loop handles that.\n\n\
Return ONLY a single JSON object with this exact shape:\n\
{\n\
  \"lesson\": {\n\
    \"condition\": \"specific situation that triggers the action\",\n\
    \"action\": \"concrete tool call recommendation, naming a tool used above\",\n\
    \"expected_outcome\": \"measurable result\",\n\
    \"confidence\": 0.0\n\
  },\n\
  \"tightenings\": [\n\
    {\n\
      \"title\": \"short imperative tightening\",\n\
      \"rationale\": \"why, grounded in the observed outcomes\",\n\
      \"current_behavior\": \"what the agent does today\",\n\
      \"proposed_constraint\": \"the guard/precondition to add — it must RESTRICT \
behavior, never relax or remove an existing restriction\",\n\
      \"severity\": \"low|medium|high\",\n\
      \"confidence\": 0.0,\n\
      \"evidence_episode_count\": 0\n\
    }\n\
  ]\n\
}\n\
Set \"lesson\" to null if no action-named pattern is clear. Use an empty \
\"tightenings\" array if no constraint is warranted. A valid lesson MUST name \
a specific action from the evidence.";

/// Render the action→outcome evidence for the user turn.
#[must_use]
pub fn build_user_prompt(batch: &CategoryBatch) -> String {
    let evidence: Vec<Value> = batch
        .outcomes
        .iter()
        .map(|o| {
            json!({
                "actions": o.actions,
                "success": o.success,
                "quality": o.quality,
            })
        })
        .collect();
    let success = batch.outcomes.iter().filter(|o| o.success).count();
    let failure = batch.outcomes.len() - success;
    let evidence_str = serde_json::to_string_pretty(&evidence).unwrap_or_default();
    format!(
        "Category: {}\n\nAction->outcome pairs ({} successes, {} failures):\n{}",
        batch.category, success, failure, evidence_str
    )
}

/// Parsed consolidation output for one category. Items that violated the
/// contract are in `rejects`, with reasons.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConsolidationOutput {
    pub lesson: Option<ActionLesson>,
    pub proposals: Vec<Proposal>,
    pub rejects: Vec<Reject>,
}

/// Slice the first balanced-looking JSON object out of a model reply and
/// neutralize control characters that break `serde_json` (LLMs emit raw
/// newlines inside string values). Mirrors the runtime synthesis parser.
fn extract_json(content: &str) -> Option<String> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end < start {
        return None;
    }
    let sanitized: String = content[start..=end]
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    Some(sanitized)
}

fn parse_lesson(category: &str, value: &Value, episode_count: u32) -> Result<ActionLesson, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "lesson is not a JSON object".to_string())?;
    let condition = obj
        .get("condition")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| "lesson missing condition".to_string())?;
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| "lesson missing action".to_string())?;
    let expected = obj
        .get("expected_outcome")
        .and_then(Value::as_str)
        .unwrap_or("improved_outcome")
        .trim();
    let confidence = obj.get("confidence").and_then(Value::as_f64).unwrap_or(0.5);
    Ok(ActionLesson::new(
        category.to_string(),
        condition.to_string(),
        action.to_string(),
        expected.to_string(),
        confidence,
        episode_count,
    ))
}

fn parse_proposal(
    category: &str,
    value: &Value,
    default_episodes: u32,
) -> Result<Proposal, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "tightening is not a JSON object".to_string())?;
    let title = obj
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "tightening missing title".to_string())?;
    let constraint = obj
        .get("proposed_constraint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| "tightening missing proposed_constraint".to_string())?;
    let severity = Severity::parse_strict(obj.get("severity").and_then(Value::as_str))?;
    let rationale = obj
        .get("rationale")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let current = obj
        .get("current_behavior")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let confidence = obj.get("confidence").and_then(Value::as_f64).unwrap_or(0.5);
    let evidence = obj
        .get("evidence_episode_count")
        .and_then(Value::as_u64)
        .map_or(default_episodes, |n| n as u32);
    Ok(Proposal::new(
        category.to_string(),
        title.to_string(),
        rationale.to_string(),
        current.to_string(),
        constraint.to_string(),
        severity,
        confidence,
        evidence,
    ))
}

/// Parse a Fable reply into lessons + candidate tightenings + rejects.
///
/// A reply that fails to parse at all becomes a single [`RejectKind::Response`]
/// record; individually malformed items become [`RejectKind::Lesson`] /
/// [`RejectKind::Tightening`] records. The run never aborts on a bad reply.
#[must_use]
pub fn parse_response(category: &str, reply: &str, episode_count: u32) -> ConsolidationOutput {
    let mut output = ConsolidationOutput::default();

    let parsed = extract_json(reply).and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let Some(root) = parsed else {
        output.rejects.push(Reject::new(
            category,
            RejectKind::Response,
            "reply does not contain a parseable JSON object".to_string(),
            Value::String(reply.to_string()),
        ));
        return output;
    };

    match root.get("lesson") {
        None | Some(Value::Null) => {}
        Some(value) => match parse_lesson(category, value, episode_count) {
            Ok(lesson) => output.lesson = Some(lesson),
            Err(reason) => output.rejects.push(Reject::new(
                category,
                RejectKind::Lesson,
                reason,
                value.clone(),
            )),
        },
    }

    if let Some(arr) = root.get("tightenings").and_then(Value::as_array) {
        for value in arr {
            match parse_proposal(category, value, episode_count) {
                Ok(proposal) => output.proposals.push(proposal),
                Err(reason) => output.rejects.push(Reject::new(
                    category,
                    RejectKind::Tightening,
                    reason,
                    value.clone(),
                )),
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episodes::ActionOutcome;
    use crate::proposal::ProposalStatus;

    fn batch() -> CategoryBatch {
        CategoryBatch {
            category: "colony".into(),
            episode_ids: vec!["ep-1".into(), "ep-2".into()],
            outcomes: vec![
                ActionOutcome {
                    actions: vec!["set_priority".into()],
                    success: true,
                    quality: 0.9,
                },
                ActionOutcome {
                    actions: vec!["reset".into()],
                    success: false,
                    quality: 0.1,
                },
            ],
        }
    }

    #[test]
    fn user_prompt_carries_evidence_and_counts() {
        let prompt = build_user_prompt(&batch());
        assert!(prompt.contains("Category: colony"));
        assert!(prompt.contains("1 successes, 1 failures"));
        assert!(prompt.contains("set_priority"));
    }

    #[test]
    fn parses_lesson_and_tightening() {
        let reply = r#"Here you go:
{
  "lesson": {
    "condition": "food is low",
    "action": "set_priority(Growing)",
    "expected_outcome": "food recovers",
    "confidence": 0.85
  },
  "tightenings": [
    {
      "title": "Guard reset",
      "rationale": "reset wiped the colony in failures",
      "current_behavior": "agent resets on reconnect",
      "proposed_constraint": "only reset when TotalReward < -20",
      "severity": "high",
      "confidence": 0.9,
      "evidence_episode_count": 4
    }
  ]
}
done"#;
        let out = parse_response("colony", reply, 6);
        let lesson = out.lesson.unwrap();
        assert_eq!(lesson.action_name, "set_priority");
        assert!((lesson.confidence - 0.85).abs() < 1e-9);
        assert_eq!(lesson.episode_count, 6);
        assert_eq!(out.proposals.len(), 1);
        let p = &out.proposals[0];
        assert_eq!(p.severity, Severity::High);
        assert_eq!(p.evidence_episode_count, 4);
        assert_eq!(p.status, ProposalStatus::Draft);
        assert!(out.rejects.is_empty());
    }

    #[test]
    fn null_lesson_and_empty_tightenings_is_conformant() {
        let reply = r#"{"lesson": null, "tightenings": []}"#;
        let out = parse_response("colony", reply, 3);
        assert!(out.lesson.is_none());
        assert!(out.proposals.is_empty());
        assert!(out.rejects.is_empty());
    }

    #[test]
    fn malformed_reply_becomes_response_reject() {
        let out = parse_response("colony", "the model refused", 3);
        assert!(out.lesson.is_none());
        assert!(out.proposals.is_empty());
        assert_eq!(out.rejects.len(), 1);
        assert_eq!(out.rejects[0].kind, RejectKind::Response);
        assert!(out.rejects[0].reason.contains("parseable"));
    }

    #[test]
    fn lesson_missing_action_becomes_lesson_reject() {
        let reply = r#"{"lesson": {"condition": "x", "action": "  "}, "tightenings": []}"#;
        let out = parse_response("colony", reply, 3);
        assert!(out.lesson.is_none());
        assert_eq!(out.rejects.len(), 1);
        assert_eq!(out.rejects[0].kind, RejectKind::Lesson);
        assert!(out.rejects[0].reason.contains("missing action"));
    }

    #[test]
    fn tightening_missing_constraint_becomes_tightening_reject() {
        let reply = r#"{"lesson": null, "tightenings": [{"title": "t", "severity": "low"}]}"#;
        let out = parse_response("colony", reply, 3);
        assert!(out.proposals.is_empty());
        assert_eq!(out.rejects.len(), 1);
        assert_eq!(out.rejects[0].kind, RejectKind::Tightening);
        assert!(out.rejects[0].reason.contains("proposed_constraint"));
    }

    #[test]
    fn unrecognized_severity_becomes_tightening_reject() {
        let reply = r#"{"lesson": null, "tightenings": [{
            "title": "Guard reset",
            "proposed_constraint": "only reset when colony dead",
            "severity": "catastrophic-ish"
        }]}"#;
        let out = parse_response("colony", reply, 3);
        assert!(out.proposals.is_empty());
        assert_eq!(out.rejects.len(), 1);
        assert!(out.rejects[0].reason.contains("severity"));
    }

    #[test]
    fn tolerates_raw_control_characters() {
        let reply = "{\"lesson\": {\"condition\": \"load\nhigh\", \"action\": \"throttle\nnow\", \"confidence\": 0.7}, \"tightenings\": []}";
        let out = parse_response("net", reply, 3);
        let lesson = out.lesson.unwrap();
        assert_eq!(lesson.condition, "load high");
        assert_eq!(lesson.action_name, "throttle");
    }
}
