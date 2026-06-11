//! Post-hoc validation of Fable output against the consolidation contract.
//!
//! The contract is enforced after the fact, not trusted: every emitted item
//! either passes validation or lands in `rejects.json` with a reason. The
//! accept/reject split is what produces the morning's contract-conformance
//! rate — the number that decides how the runtime wiring is designed.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::proposal::{ActionLesson, Proposal, SOURCE};

/// Cap on the raw payload preserved in a reject record — the full reply lives
/// in the raw response archive, the reject only needs enough to diagnose.
const PAYLOAD_CAP: usize = 2000;

/// What kind of emitted item was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectKind {
    Lesson,
    Tightening,
    /// The whole reply failed to parse as the output contract.
    Response,
    /// The call itself failed (transport or API error after retries) — an
    /// operational failure, not a contract violation, so it never counts
    /// against conformance.
    Call,
}

/// A contract violation: the offending payload plus why it was rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reject {
    pub category: String,
    pub kind: RejectKind,
    pub reason: String,
    /// The offending model output (truncated), kept for contract evolution.
    pub payload: Value,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

impl Reject {
    #[must_use]
    pub fn new(category: &str, kind: RejectKind, reason: String, payload: Value) -> Self {
        Self {
            category: category.to_string(),
            kind,
            reason,
            payload: truncate_payload(payload),
            source: SOURCE.to_string(),
            created_at: Utc::now(),
        }
    }
}

/// Truncate string payloads so rejects.json stays reviewable.
fn truncate_payload(payload: Value) -> Value {
    match payload {
        Value::String(s) if s.len() > PAYLOAD_CAP => {
            let cut: String = s.chars().take(PAYLOAD_CAP).collect();
            Value::String(format!("{cut}...(truncated from {} chars)", s.len()))
        }
        other => other,
    }
}

/// Case-insensitive action-name set from a batch's stripped evidence.
#[must_use]
pub fn evidence_actions(outcomes: &[crate::episodes::ActionOutcome]) -> BTreeSet<String> {
    outcomes
        .iter()
        .flat_map(|o| o.actions.iter())
        .map(|a| a.to_lowercase())
        .collect()
}

/// True when a lesson's action token names one of the evidence tools, allowing
/// for namespace prefixes in either direction (`reset` ↔ `game-rl:reset`).
fn names_evidence_action(token: &str, evidence: &BTreeSet<String>) -> bool {
    let t = token.to_lowercase();
    evidence
        .iter()
        .any(|e| e == &t || e.ends_with(&format!(":{t}")) || t.ends_with(&format!(":{e}")))
}

/// Validate a lesson against the batch it came from. Returns the rejection
/// reason, or `None` when the lesson conforms.
#[must_use]
pub fn lesson_violation(lesson: &ActionLesson, evidence: &BTreeSet<String>) -> Option<String> {
    if !crate::episodes::is_action_tool(&lesson.action_name) {
        return Some(format!(
            "lesson names a read-only tool {:?} — observe/list/hash/summary are \
             stripped from evidence and may not be recommended",
            lesson.action_name
        ));
    }
    if !names_evidence_action(&lesson.action_name, evidence) {
        return Some(format!(
            "lesson action {:?} does not name any action tool in the batch \
             evidence ({:?})",
            lesson.action_name,
            evidence.iter().cloned().collect::<Vec<_>>()
        ));
    }
    None
}

/// Phrases that mark a constraint as loosening rather than tightening.
const LOOSENING_PHRASES: &[&str] = &[
    "relax",
    "loosen",
    "remove the guard",
    "remove guard",
    "remove the restriction",
    "remove restriction",
    "remove the limit",
    "remove limit",
    "lift the restriction",
    "lift restriction",
    "increase the limit",
    "increase limit",
    "raise the limit",
    "raise limit",
    "allow more",
    "allow all",
    "always allow",
    "permit all",
    "disable the check",
    "disable check",
    "skip validation",
    "skip the check",
    "no longer require",
    "drop the requirement",
    "expand access",
    "widen access",
    "without restriction",
    "unrestricted",
];

/// Cues that restricting language is present. Generous on purpose: a false
/// positive here just means a human reviews the card; a false negative
/// silently passes a non-tightening through as a "tightening".
const RESTRICTIVE_CUES: &[&str] = &[
    "only",
    "never",
    "must",
    "not ",
    "n't",
    "require",
    "deny",
    "block",
    "reject",
    "limit",
    "guard",
    "unless",
    "max",
    "cap",
    "at most",
    "at least",
    "no more than",
    "forbid",
    "prohibit",
    "restrict",
    "exclusively",
    "solely",
    "before",
    "verify",
    "confirm",
    "check",
    "validate",
    "gate",
    "<",
    ">",
];

/// Validate that a proposal's constraint actually tightens. Returns the
/// rejection reason, or `None` when it conforms.
#[must_use]
pub fn tightening_violation(proposal: &Proposal) -> Option<String> {
    let text = proposal.proposed_constraint.to_lowercase();
    if let Some(phrase) = LOOSENING_PHRASES.iter().find(|p| text.contains(*p)) {
        return Some(format!(
            "proposed constraint loosens rather than tightens (contains {phrase:?})"
        ));
    }
    if !RESTRICTIVE_CUES.iter().any(|c| text.contains(c)) {
        return Some(
            "proposed constraint contains no restricting language — cannot be \
             reviewed as a tightening"
                .to_string(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episodes::ActionOutcome;
    use crate::proposal::Severity;

    fn evidence() -> BTreeSet<String> {
        evidence_actions(&[
            ActionOutcome {
                actions: vec!["game-rl:reset".into(), "set_priority".into()],
                success: false,
                quality: 0.1,
            },
            ActionOutcome {
                actions: vec!["set_priority".into()],
                success: true,
                quality: 0.9,
            },
        ])
    }

    fn lesson(action: &str) -> ActionLesson {
        ActionLesson::new(
            "colony".into(),
            "food low".into(),
            action.into(),
            "food recovers".into(),
            0.8,
            5,
        )
    }

    fn proposal(constraint: &str) -> Proposal {
        Proposal::new(
            "colony".into(),
            "Guard reset".into(),
            "reset wiped the colony".into(),
            "agent resets on reconnect".into(),
            constraint.into(),
            Severity::High,
            0.9,
            5,
        )
    }

    #[test]
    fn lesson_naming_evidence_action_passes() {
        assert!(lesson_violation(&lesson("set_priority(Growing)"), &evidence()).is_none());
        // Namespace-tolerant in both directions.
        assert!(lesson_violation(&lesson("reset"), &evidence()).is_none());
        assert!(lesson_violation(&lesson("game-rl:set_priority"), &evidence()).is_none());
    }

    #[test]
    fn lesson_naming_unknown_action_rejected() {
        let reason = lesson_violation(&lesson("teleport"), &evidence()).unwrap();
        assert!(reason.contains("teleport"));
    }

    #[test]
    fn lesson_naming_read_only_tool_rejected() {
        let reason = lesson_violation(&lesson("observe first"), &evidence()).unwrap();
        assert!(reason.contains("read-only"));
    }

    #[test]
    fn tightening_with_restrictive_language_passes() {
        assert!(tightening_violation(&proposal("only reset when TotalReward < -20")).is_none());
        assert!(tightening_violation(&proposal("must not reset on reconnect")).is_none());
    }

    #[test]
    fn loosening_constraint_rejected() {
        let reason =
            tightening_violation(&proposal("relax the reset guard to allow more retries")).unwrap();
        assert!(reason.contains("loosens"));
        assert!(tightening_violation(&proposal("always allow reset")).is_some());
    }

    #[test]
    fn constraint_without_restricting_language_rejected() {
        let reason = tightening_violation(&proposal("reset behavior is fine")).unwrap();
        assert!(reason.contains("no restricting language"));
    }

    #[test]
    fn reject_payload_truncated() {
        let long = "x".repeat(5000);
        let reject = Reject::new(
            "colony",
            RejectKind::Response,
            "unparseable".into(),
            Value::String(long),
        );
        let s = reject.payload.as_str().unwrap();
        assert!(s.len() < 2100);
        assert!(s.contains("truncated from 5000"));
    }
}
