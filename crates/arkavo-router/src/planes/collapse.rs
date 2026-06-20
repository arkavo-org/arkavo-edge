//! Answer adequacy gate — a hard, unsolved product problem.
//!
//! This module is deliberately NOT named a "quality gate". v1 cannot judge
//! whether an answer is *good*; it can only catch visible *collapse*. It is a
//! collapse detector. A [`CollapseVerdict::NoVisibleCollapse`] result means the
//! output "did not visibly fall apart" — it is **not** a guarantee of adequacy.
//!
//! A detected collapse may *request* a cloud upgrade offer. It may never
//! authorize cloud spend; that is the budget (spend) plane's sole job. The
//! honest v1 policy is: local answer completed, cloud upgrade available, ask
//! the user before spending — never "local answer seems bad, auto-spend cloud".

use serde::{Deserialize, Serialize};

/// A visible collapse mode. These are the only failures v1 can reliably detect;
/// they are signs of breakdown, not a measure of answer quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollapseSignal {
    /// No usable output was produced.
    EmptyOutput,
    /// Generation was cut off by the output-token cap.
    TruncatedOutput,
    /// Output degenerated into a repeating loop.
    RepetitionLoop,
    /// Output was expected to match a structured format and did not.
    FormatFailure,
    /// A tool call was emitted but failed schema validation.
    ToolCallSchemaFailure,
    /// The model refused although the task is allowed.
    DisallowedRefusal,
    /// The model plainly ignored the instructions (e.g. produced neither the
    /// required tool call nor any answer).
    InstructionNoncompliance,
    /// The model generated this answer with low token-level confidence (mean
    /// log-probability well below the confident range). This is the closest v1
    /// has to an *adequacy* signal — uncertainty, not visible breakdown — and
    /// is weaker than the structural collapses above.
    LowConfidence,
}

/// The collapse detector's verdict on a completed local answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollapseVerdict {
    /// No visible collapse. NOT a guarantee that the answer is adequate.
    NoVisibleCollapse,
    /// The output visibly collapsed in the named way.
    Collapsed(CollapseSignal),
}

impl CollapseVerdict {
    pub fn collapsed(&self) -> bool {
        matches!(self, Self::Collapsed(_))
    }

    pub fn signal(&self) -> Option<CollapseSignal> {
        match self {
            Self::Collapsed(s) => Some(*s),
            Self::NoVisibleCollapse => None,
        }
    }
}

/// Observations about a completed local answer.
///
/// The collapse modes the detector cannot derive from raw text — tool-schema
/// validity, structured-format match, refusal-vs-allowed — are computed by the
/// routing loop and passed in via [`AnswerObservation::precomputed`].
#[derive(Debug, Clone, Default)]
pub struct AnswerObservation<'a> {
    /// The visible answer text (tool/think blocks already stripped).
    pub text: &'a str,
    /// Generation stopped because it hit the output-token cap.
    pub hit_output_cap: bool,
    /// The task required a tool call (function-calling) to proceed.
    pub tool_call_required: bool,
    /// A collapse the caller already detected from context the detector cannot
    /// see (e.g. [`CollapseSignal::ToolCallSchemaFailure`],
    /// [`CollapseSignal::FormatFailure`], [`CollapseSignal::DisallowedRefusal`]).
    pub precomputed: Option<CollapseSignal>,
    /// Mean per-token log-probability of the generated answer, when the local
    /// engine reported it (`InferenceTiming.avg_logprob`). `None` for providers
    /// that don't surface logprobs. Values near `0` are confident; very negative
    /// values mean the model was guessing.
    pub avg_logprob: Option<f32>,
}

/// Mean log-probability below which the answer is flagged as low-confidence.
/// −3.0 nats ≈ an average chosen-token probability of ~5%, i.e. the model was
/// consistently unsure. Conservative so a confident answer never trips it.
const LOW_CONFIDENCE_LOGPROB: f32 = -3.0;

/// Detect visible collapse in a completed local answer.
///
/// The first matching signal wins. A caller-supplied `precomputed` signal takes
/// priority, since it reflects context the text alone cannot reveal.
pub fn detect(obs: &AnswerObservation<'_>) -> CollapseVerdict {
    if let Some(signal) = obs.precomputed {
        return CollapseVerdict::Collapsed(signal);
    }

    let trimmed = obs.text.trim();

    if trimmed.is_empty() {
        // Tool-required tasks that produced neither a valid tool call nor text
        // ignored the instruction; a plain chat turn just came back empty.
        return if obs.tool_call_required {
            CollapseVerdict::Collapsed(CollapseSignal::InstructionNoncompliance)
        } else {
            CollapseVerdict::Collapsed(CollapseSignal::EmptyOutput)
        };
    }

    if obs.hit_output_cap {
        return CollapseVerdict::Collapsed(CollapseSignal::TruncatedOutput);
    }

    if has_repetition_loop(trimmed) {
        return CollapseVerdict::Collapsed(CollapseSignal::RepetitionLoop);
    }

    // Weakest signal, checked last so it never masks a structural collapse: the
    // model produced fluent text but with consistently low token confidence.
    if obs
        .avg_logprob
        .is_some_and(|lp| lp.is_finite() && lp < LOW_CONFIDENCE_LOGPROB)
    {
        return CollapseVerdict::Collapsed(CollapseSignal::LowConfidence);
    }

    CollapseVerdict::NoVisibleCollapse
}

/// Conservative repetition-loop check: flags an answer whose tail is a short
/// phrase (1–4 words) repeated many times — the classic local-model collapse.
///
/// Known blind spots, acceptable for a v1 collapse detector that must not
/// over-fire: it only inspects the final `cycle * MIN_REPEATS` words with cycle
/// length ≤ 4, so a longer-period loop (5+ word phrase), a mid-answer loop that
/// recovers with a non-repeating tail, or a loop shorter than `MIN_REPEATS`
/// reads as `NoVisibleCollapse`. It can also false-positive on a naturally
/// emphatic repeated tail. A clean verdict means "did not visibly collapse",
/// not "the answer is good" — callers must not over-trust it.
fn has_repetition_loop(text: &str) -> bool {
    const MIN_REPEATS: usize = 6;
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < MIN_REPEATS {
        return false;
    }
    for cycle in 1..=4 {
        if words.len() < cycle * MIN_REPEATS {
            continue;
        }
        let tail = &words[words.len() - cycle * MIN_REPEATS..];
        let pattern = &tail[..cycle];
        if tail.chunks(cycle).all(|chunk| chunk == pattern) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(text: &str) -> AnswerObservation<'_> {
        AnswerObservation {
            text,
            ..Default::default()
        }
    }

    #[test]
    fn healthy_answer_has_no_visible_collapse() {
        let verdict = detect(&answer("The capital of France is Paris."));
        assert_eq!(verdict, CollapseVerdict::NoVisibleCollapse);
        assert!(!verdict.collapsed());
    }

    #[test]
    fn empty_chat_answer_is_empty_output() {
        assert_eq!(
            detect(&answer("   ")),
            CollapseVerdict::Collapsed(CollapseSignal::EmptyOutput)
        );
    }

    #[test]
    fn empty_when_tool_required_is_noncompliance() {
        let obs = AnswerObservation {
            text: "",
            tool_call_required: true,
            ..Default::default()
        };
        assert_eq!(
            detect(&obs),
            CollapseVerdict::Collapsed(CollapseSignal::InstructionNoncompliance)
        );
    }

    #[test]
    fn refusal_on_allowed_task_collapses() {
        let obs = AnswerObservation {
            text: "I cannot help with that.",
            precomputed: Some(CollapseSignal::DisallowedRefusal),
            ..Default::default()
        };
        assert_eq!(
            detect(&obs),
            CollapseVerdict::Collapsed(CollapseSignal::DisallowedRefusal)
        );
    }

    #[test]
    fn invalid_tool_schema_collapses() {
        let obs = AnswerObservation {
            text: "{\"tool\": ...}",
            precomputed: Some(CollapseSignal::ToolCallSchemaFailure),
            ..Default::default()
        };
        assert_eq!(
            detect(&obs),
            CollapseVerdict::Collapsed(CollapseSignal::ToolCallSchemaFailure)
        );
    }

    #[test]
    fn format_failure_collapses() {
        let obs = AnswerObservation {
            text: "not json",
            precomputed: Some(CollapseSignal::FormatFailure),
            ..Default::default()
        };
        assert_eq!(
            detect(&obs),
            CollapseVerdict::Collapsed(CollapseSignal::FormatFailure)
        );
    }

    #[test]
    fn output_cap_truncation_collapses() {
        let obs = AnswerObservation {
            text: "A long answer that ran out of room",
            hit_output_cap: true,
            ..Default::default()
        };
        assert_eq!(
            detect(&obs),
            CollapseVerdict::Collapsed(CollapseSignal::TruncatedOutput)
        );
    }

    #[test]
    fn repetition_loop_collapses() {
        let obs = answer("ok the answer is yes yes yes yes yes yes yes yes");
        assert_eq!(
            detect(&obs),
            CollapseVerdict::Collapsed(CollapseSignal::RepetitionLoop)
        );
    }

    #[test]
    fn multiword_repetition_loop_collapses() {
        let obs = answer("go on go on go on go on go on go on go on");
        assert_eq!(
            detect(&obs),
            CollapseVerdict::Collapsed(CollapseSignal::RepetitionLoop)
        );
    }

    #[test]
    fn ordinary_repetition_is_not_a_loop() {
        // A few natural repeats must not trip the detector.
        let obs = answer("very very good work overall, nicely done and complete");
        assert_eq!(detect(&obs), CollapseVerdict::NoVisibleCollapse);
    }

    #[test]
    fn low_token_confidence_flags_collapse() {
        let obs = AnswerObservation {
            text: "A fluent but low-confidence guess.",
            avg_logprob: Some(-3.5),
            ..Default::default()
        };
        assert_eq!(
            detect(&obs),
            CollapseVerdict::Collapsed(CollapseSignal::LowConfidence)
        );
    }

    #[test]
    fn confident_answer_is_not_low_confidence() {
        let obs = AnswerObservation {
            text: "The capital of France is Paris.",
            avg_logprob: Some(-0.4),
            ..Default::default()
        };
        assert_eq!(detect(&obs), CollapseVerdict::NoVisibleCollapse);
    }

    #[test]
    fn low_confidence_never_masks_a_structural_collapse() {
        // Empty output wins over low confidence (structural beats uncertainty).
        let obs = AnswerObservation {
            text: "   ",
            avg_logprob: Some(-9.0),
            ..Default::default()
        };
        assert_eq!(
            detect(&obs),
            CollapseVerdict::Collapsed(CollapseSignal::EmptyOutput)
        );
    }

    #[test]
    fn missing_logprob_is_not_low_confidence() {
        let obs = answer("No logprob reported by this provider.");
        assert_eq!(detect(&obs), CollapseVerdict::NoVisibleCollapse);
    }
}
