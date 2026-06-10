//! Draft output contract for shadow consolidation.
//!
//! These types are the wire shape the overnight job writes to disk and the
//! shape a future runtime / findings-inbox UI is expected to read. They are
//! deliberately self-contained (no dependency on any runtime crate) so the
//! audit-plane job and the contract it validates stay decoupled by
//! construction.
//!
//! Three artifacts map onto these types:
//!   * [`ActionLesson`] — action-named lessons.
//!   * [`Proposal`] — candidate tightenings (the draft `proposal.rs` shape).
//!   * [`CostLedger`] — the cost ledger that produces real budget numbers.
//!
//! [`FindingCard`] is the seed-card projection of the first two, so the
//! findings inbox can be populated from genuine consolidation output instead
//! of invented fixtures.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifier the job stamps on everything it emits, so downstream consumers
/// can tell shadow-consolidation output apart from runtime-synthesized data.
pub const SOURCE: &str = "fable-shadow-consolidation";

/// How urgently a candidate tightening should be reviewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Medium,
    High,
}

impl Severity {
    /// Parse a model-emitted severity string strictly. Only the contract
    /// values `low | medium | high` (plus `critical` as a high alias) are
    /// accepted; anything else is a contract violation the caller must record
    /// as a reject — leniency here would hide non-conformance from the
    /// morning's contract-conformance rate.
    pub fn parse_strict(raw: Option<&str>) -> Result<Self, String> {
        match raw.map(str::trim).map(str::to_lowercase).as_deref() {
            Some("low") => Ok(Self::Low),
            Some("medium") => Ok(Self::Medium),
            Some("high" | "critical") => Ok(Self::High),
            Some(other) => Err(format!("unrecognized severity {other:?}")),
            None => Err("missing severity".to_string()),
        }
    }
}

/// Review state of a [`Proposal`]. Everything the job emits starts as
/// [`ProposalStatus::Draft`]; acceptance/rejection is a human or runtime
/// decision made later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Draft,
    Accepted,
    Rejected,
}

/// An action-named lesson: a strategic rule that names a concrete action.
///
/// Mirrors the runtime lesson contract (condition / action / expected_outcome /
/// confidence) but is generated offline from existing traces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionLesson {
    pub id: Uuid,
    /// Task category the lesson was distilled from.
    pub category: String,
    /// The named action this lesson is about (leading token of `action`),
    /// so the inbox can group lessons by the action they recommend.
    pub action_name: String,
    /// Situation that triggers the action.
    pub condition: String,
    /// Concrete action recommendation.
    pub action: String,
    /// Measurable result the action is expected to produce.
    pub expected_outcome: String,
    /// Model confidence in the lesson, clamped to `[0, 1]`.
    pub confidence: f64,
    /// Number of episodes that backed this lesson.
    pub episode_count: u32,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

impl ActionLesson {
    /// Build a lesson, deriving [`ActionLesson::action_name`] from the leading
    /// token of `action` (the tool name, before any space or `(`).
    #[must_use]
    pub fn new(
        category: String,
        condition: String,
        action: String,
        expected_outcome: String,
        confidence: f64,
        episode_count: u32,
    ) -> Self {
        let action_name = action_token(&action);
        Self {
            id: Uuid::new_v4(),
            category,
            action_name,
            condition,
            action,
            expected_outcome,
            confidence: confidence.clamp(0.0, 1.0),
            episode_count,
            source: SOURCE.to_string(),
            created_at: Utc::now(),
        }
    }
}

/// Extract the action-naming token from a free-form action string: the part
/// before the first whitespace or `(`. Falls back to the trimmed whole string.
fn action_token(action: &str) -> String {
    let trimmed = action.trim();
    let end = trimmed
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(trimmed.len());
    let token = trimmed[..end].trim_end_matches([':', ',', '.']);
    if token.is_empty() {
        trimmed.to_string()
    } else {
        token.to_string()
    }
}

/// A candidate tightening — the draft `proposal.rs` JSON shape. Describes a
/// policy constraint Fable judged worth adding, given the action→outcome
/// evidence in a category.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub id: Uuid,
    pub category: String,
    /// Short imperative title of the tightening.
    pub title: String,
    /// Why the tightening is warranted, grounded in the observed outcomes.
    pub rationale: String,
    /// The behavior as it stands today (what the agent currently does).
    pub current_behavior: String,
    /// The constraint to add — the actual tightening.
    pub proposed_constraint: String,
    pub severity: Severity,
    /// Model confidence in the tightening, clamped to `[0, 1]`.
    pub confidence: f64,
    /// How many episodes back this proposal.
    pub evidence_episode_count: u32,
    pub status: ProposalStatus,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

impl Proposal {
    #[must_use]
    #[allow(clippy::too_many_arguments)] // plain field-by-field constructor
    pub fn new(
        category: String,
        title: String,
        rationale: String,
        current_behavior: String,
        proposed_constraint: String,
        severity: Severity,
        confidence: f64,
        evidence_episode_count: u32,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            category,
            title,
            rationale,
            current_behavior,
            proposed_constraint,
            severity,
            confidence: confidence.clamp(0.0, 1.0),
            evidence_episode_count,
            status: ProposalStatus::Draft,
            source: SOURCE.to_string(),
            created_at: Utc::now(),
        }
    }
}

/// What a [`FindingCard`] was projected from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Lesson,
    Tightening,
}

/// A seed card for the findings-inbox UI, projected from a real lesson or
/// proposal so the inbox shows genuine consolidation output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingCard {
    pub id: Uuid,
    pub kind: FindingKind,
    pub title: String,
    pub body: String,
    pub category: String,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    /// Back-link to the source artifact, so accepting a card can act on the
    /// underlying lesson or proposal.
    pub source_id: Uuid,
    pub source: String,
    pub created_at: DateTime<Utc>,
}

impl FindingCard {
    #[must_use]
    pub fn from_lesson(lesson: &ActionLesson) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind: FindingKind::Lesson,
            title: format!("Lesson: {}", lesson.action_name),
            body: format!(
                "When {}, {} (expect: {}).",
                lesson.condition.trim(),
                lesson.action.trim(),
                lesson.expected_outcome.trim()
            ),
            category: lesson.category.clone(),
            confidence: lesson.confidence,
            severity: None,
            source_id: lesson.id,
            source: SOURCE.to_string(),
            created_at: Utc::now(),
        }
    }

    #[must_use]
    pub fn from_proposal(proposal: &Proposal) -> Self {
        Self {
            id: Uuid::new_v4(),
            kind: FindingKind::Tightening,
            title: format!("Tightening: {}", proposal.title.trim()),
            body: format!(
                "{} Constraint: {}",
                proposal.rationale.trim(),
                proposal.proposed_constraint.trim()
            ),
            category: proposal.category.clone(),
            confidence: proposal.confidence,
            severity: Some(proposal.severity),
            source_id: proposal.id,
            source: SOURCE.to_string(),
            created_at: Utc::now(),
        }
    }
}

/// Per-call accounting row in the [`CostLedger`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub category: String,
    /// Store row ids of the episodes behind this batch — the batch→episode
    /// linkage that lets a proposal be backfilled into trace references and
    /// findings-inbox cards keep their evidence.
    pub episode_ids: Vec<String>,
    pub episode_count: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub latency_ms: u64,
    pub lessons: u32,
    pub tightenings: u32,
}

/// The empirical budget number this whole job exists to produce: a concrete
/// value to set `budget.per_layer["consolidation"]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetRecommendation {
    /// Layer key the recommendation is for.
    pub layer: String,
    /// Suggested per-invocation ceiling — the unit `LayerBudget.limit_usd`
    /// uses (a single consolidation completion). Max observed call cost with
    /// headroom.
    pub suggested_per_invocation_limit_usd: f64,
    /// Suggested ceiling for one full overnight run across all categories.
    pub suggested_per_run_total_usd: f64,
    /// Plain-language explanation of how the numbers were derived.
    pub basis: String,
}

/// The cost ledger: real token and dollar accounting for the run, plus the
/// derived budget recommendation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostLedger {
    pub model: String,
    pub generated_at: DateTime<Utc>,
    pub dry_run: bool,
    pub episodes_total: u64,
    pub episodes_consolidated: u64,
    pub categories_total: u64,
    pub categories_consolidated: u64,
    pub categories_skipped: u64,
    pub categories_budget_stopped: u64,
    pub run_budget: RunBudget,
    pub conformance: Conformance,
    /// Headline metric: share of model-emitted items that conformed to the
    /// output contract ([`Conformance::rate`]).
    pub contract_conformance_rate: f64,
    pub fable_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_cost_usd: f64,
    pub max_call_cost_usd: f64,
    pub mean_call_cost_usd: f64,
    pub per_call: Vec<LedgerEntry>,
    pub budget_recommendation: BudgetRecommendation,
}

/// Round a dollar amount up to whole cents, so a recommended ceiling never
/// sits a fraction of a cent below an observed cost.
fn ceil_cents(usd: f64) -> f64 {
    (usd * 100.0).ceil() / 100.0
}

/// Episode/category tallies for a run, threaded into [`CostLedger::summarize`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStats {
    pub episodes_total: u64,
    pub episodes_consolidated: u64,
    pub categories_total: u64,
    pub categories_skipped: u64,
    /// Categories left unconsolidated because the run-level cost ceiling was
    /// hit before they were reached.
    pub categories_budget_stopped: u64,
}

/// Accepted/rejected tallies for the run — the source of the morning's
/// contract-conformance rate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conformance {
    pub lessons_accepted: u64,
    pub lessons_rejected: u64,
    pub tightenings_accepted: u64,
    pub tightenings_rejected: u64,
    /// Replies that could not be parsed as the output contract at all.
    pub responses_rejected: u64,
}

impl Conformance {
    /// Share of emitted items that conformed to the output contract.
    /// `1.0` when the model emitted nothing (an empty run is not a violation).
    #[must_use]
    pub fn rate(&self) -> f64 {
        let accepted = self.lessons_accepted + self.tightenings_accepted;
        let rejected = self.lessons_rejected + self.tightenings_rejected + self.responses_rejected;
        let total = accepted + rejected;
        if total == 0 {
            1.0
        } else {
            accepted as f64 / total as f64
        }
    }
}

/// Run-level spend ceiling state, recorded in the ledger.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RunBudget {
    /// The hard ceiling the run was given (`--max-run-cost-usd`).
    pub max_run_cost_usd: f64,
    /// True when the run stopped early because cumulative spend reached the
    /// ceiling; the artifacts on disk are then partial by design.
    pub exhausted: bool,
}

impl CostLedger {
    /// Aggregate per-call entries into a ledger and derive the budget
    /// recommendation. `headroom` (e.g. `1.5`) is applied to the worst-observed
    /// call cost to set the per-invocation ceiling.
    #[must_use]
    pub fn summarize(
        model: String,
        dry_run: bool,
        stats: RunStats,
        conformance: Conformance,
        run_budget: RunBudget,
        entries: Vec<LedgerEntry>,
        headroom: f64,
    ) -> Self {
        let fable_calls = entries.len() as u64;
        let input_tokens = entries.iter().map(|e| e.input_tokens).sum();
        let output_tokens = entries.iter().map(|e| e.output_tokens).sum();
        // `+ 0.0` normalizes the `-0.0` that f64's empty Sum identity yields,
        // which would otherwise surface as "$-0.0000" in reports.
        let total_cost_usd: f64 = entries.iter().map(|e| e.cost_usd).sum::<f64>() + 0.0;
        let max_call_cost_usd = entries.iter().map(|e| e.cost_usd).fold(0.0_f64, f64::max);
        let mean_call_cost_usd = if fable_calls == 0 {
            0.0
        } else {
            total_cost_usd / fable_calls as f64
        };

        let per_invocation = ceil_cents(max_call_cost_usd * headroom);
        let per_run = ceil_cents(total_cost_usd * 1.25);
        let basis = format!(
            "{fable_calls} Fable call(s) over {} episode(s); \
             worst call ${max_call_cost_usd:.4}, mean ${mean_call_cost_usd:.4}, \
             run total ${total_cost_usd:.4}. Per-invocation ceiling = worst call \
             x {headroom} headroom; per-run ceiling = total x 1.25.",
            stats.episodes_consolidated
        );

        Self {
            model,
            generated_at: Utc::now(),
            dry_run,
            episodes_total: stats.episodes_total,
            episodes_consolidated: stats.episodes_consolidated,
            categories_total: stats.categories_total,
            categories_consolidated: fable_calls,
            categories_skipped: stats.categories_skipped,
            categories_budget_stopped: stats.categories_budget_stopped,
            run_budget,
            contract_conformance_rate: conformance.rate(),
            conformance,
            fable_calls,
            input_tokens,
            output_tokens,
            total_cost_usd,
            max_call_cost_usd,
            mean_call_cost_usd,
            per_call: entries,
            budget_recommendation: BudgetRecommendation {
                layer: "consolidation".to_string(),
                suggested_per_invocation_limit_usd: per_invocation,
                suggested_per_run_total_usd: per_run,
                basis,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_token_takes_leading_tool_name() {
        assert_eq!(action_token("game-rl:step(Action=...)"), "game-rl:step");
        assert_eq!(action_token("  set_priority high  "), "set_priority");
        assert_eq!(action_token("reset:"), "reset");
    }

    #[test]
    fn action_token_falls_back_to_whole_string() {
        assert_eq!(action_token("avoid"), "avoid");
        assert_eq!(action_token("((("), "(((");
    }

    #[test]
    fn lesson_derives_action_name_and_clamps_confidence() {
        let lesson = ActionLesson::new(
            "colony".into(),
            "food low".into(),
            "set_priority(Growing)".into(),
            "food recovers".into(),
            1.7,
            5,
        );
        assert_eq!(lesson.action_name, "set_priority");
        assert!((lesson.confidence - 1.0).abs() < f64::EPSILON);
        assert_eq!(lesson.source, SOURCE);
    }

    #[test]
    fn severity_parse_is_strict() {
        assert_eq!(Severity::parse_strict(Some("LOW")).unwrap(), Severity::Low);
        assert_eq!(
            Severity::parse_strict(Some("medium")).unwrap(),
            Severity::Medium
        );
        assert_eq!(
            Severity::parse_strict(Some("critical")).unwrap(),
            Severity::High
        );
        assert!(Severity::parse_strict(Some("weird")).is_err());
        assert!(Severity::parse_strict(None).is_err());
    }

    #[test]
    fn conformance_rate_over_emitted_items() {
        let c = Conformance {
            lessons_accepted: 3,
            lessons_rejected: 1,
            tightenings_accepted: 4,
            tightenings_rejected: 1,
            responses_rejected: 1,
        };
        // 7 accepted / 10 total
        assert!((c.rate() - 0.7).abs() < 1e-9);
        assert!((Conformance::default().rate() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn proposal_starts_as_draft() {
        let p = Proposal::new(
            "colony".into(),
            "Guard reset".into(),
            "reset wipes colony".into(),
            "agent resets on reconnect".into(),
            "only reset when TotalReward < -20".into(),
            Severity::High,
            0.9,
            7,
        );
        assert_eq!(p.status, ProposalStatus::Draft);
        assert_eq!(p.severity, Severity::High);
    }

    #[test]
    fn finding_cards_back_link_to_source() {
        let lesson = ActionLesson::new(
            "colony".into(),
            "food low".into(),
            "set_priority(Growing)".into(),
            "food recovers".into(),
            0.8,
            5,
        );
        let card = FindingCard::from_lesson(&lesson);
        assert_eq!(card.kind, FindingKind::Lesson);
        assert_eq!(card.source_id, lesson.id);
        assert!(card.title.contains("set_priority"));

        let proposal = Proposal::new(
            "colony".into(),
            "Guard reset".into(),
            "reset wipes colony".into(),
            "agent resets on reconnect".into(),
            "only reset when TotalReward < -20".into(),
            Severity::High,
            0.9,
            7,
        );
        let pcard = FindingCard::from_proposal(&proposal);
        assert_eq!(pcard.kind, FindingKind::Tightening);
        assert_eq!(pcard.severity, Some(Severity::High));
        assert_eq!(pcard.source_id, proposal.id);
    }

    #[test]
    fn ledger_derives_budget_from_worst_call() {
        let entries = vec![
            LedgerEntry {
                category: "a".into(),
                episode_ids: vec!["ep-1".into(), "ep-2".into(), "ep-3".into()],
                episode_count: 3,
                input_tokens: 1000,
                output_tokens: 500,
                cost_usd: 0.0350,
                latency_ms: 1200,
                lessons: 1,
                tightenings: 2,
            },
            LedgerEntry {
                category: "b".into(),
                episode_ids: vec!["ep-4".into(); 4],
                episode_count: 4,
                input_tokens: 2000,
                output_tokens: 800,
                cost_usd: 0.0600,
                latency_ms: 1500,
                lessons: 1,
                tightenings: 0,
            },
        ];
        let stats = RunStats {
            episodes_total: 20,
            episodes_consolidated: 7,
            categories_total: 5,
            categories_skipped: 3,
            categories_budget_stopped: 0,
        };
        let conformance = Conformance {
            lessons_accepted: 2,
            tightenings_accepted: 2,
            ..Conformance::default()
        };
        let run_budget = RunBudget {
            max_run_cost_usd: 25.0,
            exhausted: false,
        };
        let ledger = CostLedger::summarize(
            "claude-fable-5".into(),
            false,
            stats,
            conformance,
            run_budget,
            entries,
            1.5,
        );
        assert_eq!(ledger.fable_calls, 2);
        assert_eq!(ledger.per_call[0].episode_ids.len(), 3);
        assert!((ledger.contract_conformance_rate - 1.0).abs() < f64::EPSILON);
        assert!(!ledger.run_budget.exhausted);
        assert_eq!(ledger.input_tokens, 3000);
        assert_eq!(ledger.output_tokens, 1300);
        assert!((ledger.total_cost_usd - 0.095).abs() < 1e-9);
        assert!((ledger.max_call_cost_usd - 0.06).abs() < 1e-9);
        // worst call 0.06 * 1.5 = 0.09, ceil to cents = 0.09
        assert!(
            (ledger
                .budget_recommendation
                .suggested_per_invocation_limit_usd
                - 0.09)
                .abs()
                < 1e-9
        );
        assert_eq!(ledger.budget_recommendation.layer, "consolidation");
    }

    #[test]
    fn ledger_empty_run_is_zeroed() {
        let stats = RunStats {
            episodes_total: 0,
            episodes_consolidated: 0,
            categories_total: 0,
            categories_skipped: 0,
            categories_budget_stopped: 0,
        };
        let ledger = CostLedger::summarize(
            "claude-fable-5".into(),
            true,
            stats,
            Conformance::default(),
            RunBudget {
                max_run_cost_usd: 25.0,
                exhausted: false,
            },
            vec![],
            1.5,
        );
        assert_eq!(ledger.fable_calls, 0);
        assert!((ledger.mean_call_cost_usd).abs() < f64::EPSILON);
        assert!(
            (ledger
                .budget_recommendation
                .suggested_per_invocation_limit_usd)
                .abs()
                < f64::EPSILON
        );
    }
}
