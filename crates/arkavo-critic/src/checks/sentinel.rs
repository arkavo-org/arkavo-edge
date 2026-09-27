//! The sentinel as a pipeline check (SENT-014).
//!
//! It contributes evidence and never a verdict. The distinction is the whole
//! design: a check that could fail the pipeline would be a second policy
//! engine, disagreeing with the first at exactly the moments that matter. So
//! this check returns `Warn` when labels fire and `Pass` when they do not, and
//! the policy layer reads the evidence off the pipeline result.
//!
//! It runs after the circuit check and is skipped once anything has failed:
//! a block is not softened by later evidence, and a blocked action has nothing
//! left for evidence to inform.
//!
//! The classifier arrives through [`ClassificationSource`] rather than as a
//! direct dependency. That is not indirection for its own sake — this crate
//! sits underneath the router, and the cascade sits above the protocol crate
//! the router is reachable from, so a direct dependency would close a cycle.
//! The wiring crate, which is above both, supplies the adapter.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Instant;

use crate::checks::{CheckResult, VerificationCheck, VerificationInput};
use crate::evidence::VerificationEvidence;

/// Priority placing this after the circuit check's default of 5 and before the
/// checks that run at 25 and above.
pub const SENTINEL_PRIORITY: u32 = 10;

/// What a classification cascade reported about a span.
///
/// Counts and a gap flag, plus the full evidence as opaque detail. The check
/// deliberately cannot interpret `details`: anything it could branch on would
/// be a decision, and decisions are the policy layer's.
#[derive(Debug, Clone, Default)]
pub struct SentinelEvidence {
    /// Labels that fired across every tier.
    pub labels: usize,
    /// Tiers consulted, including those that found nothing.
    pub tiers: usize,
    /// Whether a tier could not answer, which is not a clean result.
    pub has_gap: bool,
    /// The full evidence contract, carried through to the pipeline result.
    pub details: serde_json::Value,
}

/// A classifier the check can consult.
pub trait ClassificationSource: Send + Sync {
    fn inspect(&self, text: &str) -> SentinelEvidence;
}

pub struct SentinelCheck {
    source: Arc<dyn ClassificationSource>,
    priority: u32,
}

impl SentinelCheck {
    pub fn new(source: Arc<dyn ClassificationSource>) -> Self {
        Self {
            source,
            priority: SENTINEL_PRIORITY,
        }
    }

    #[must_use]
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.priority = priority;
        self
    }

    /// Everything the response would put somewhere: the completion text and
    /// every tool-call argument, inspected whole before anything runs.
    fn inspectable(input: &VerificationInput) -> String {
        let mut text = input.response.content.clone();
        if let Some(reasoning) = &input.response.reasoning_content {
            text.push('\n');
            text.push_str(reasoning);
        }
        for call in &input.response.tool_calls {
            text.push('\n');
            text.push_str(&call.arguments.to_string());
        }
        text
    }
}

#[async_trait]
impl VerificationCheck for SentinelCheck {
    fn id(&self) -> &'static str {
        "sentinel"
    }

    fn priority(&self) -> u32 {
        self.priority
    }

    fn skip_after_failure(&self) -> bool {
        true
    }

    async fn verify(&self, input: &VerificationInput) -> CheckResult {
        let started = Instant::now();
        let source = self.source.clone();
        let text = Self::inspectable(input);
        let evidence = tokio::task::spawn_blocking(move || source.inspect(&text))
            .await
            .unwrap_or_else(|_| SentinelEvidence {
                has_gap: true,
                ..Default::default()
            });
        let latency_us = started.elapsed().as_micros() as u64;

        if evidence.labels == 0 && !evidence.has_gap {
            return CheckResult::Pass;
        }

        // Warn, never Fail: this check reports what the tiers saw. Whether that
        // blocks anything is the policy layer's decision, made from the
        // evidence attached here.
        CheckResult::Warn(
            VerificationEvidence::partial(
                self.id(),
                // A gap is not a clean pass and not a score of its own; the
                // score reports coverage and the details carry the reason.
                if evidence.has_gap { 0.0 } else { 1.0 },
                &format!(
                    "{} label(s) from {} tier(s)",
                    evidence.labels, evidence.tiers
                ),
                latency_us,
            )
            .with_details(evidence.details),
        )
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::checks::CircuitCheck;
    use crate::pipeline::CriticPipeline;
    use arkavo_llm::ProviderResponse;
    use arkavo_test_macros::spec;

    struct Fixed(SentinelEvidence);

    impl ClassificationSource for Fixed {
        fn inspect(&self, _text: &str) -> SentinelEvidence {
            self.0.clone()
        }
    }

    fn input(content: &str) -> VerificationInput {
        VerificationInput::new(
            "task".into(),
            ProviderResponse {
                content: content.to_string(),
                reasoning_content: None,
                tool_calls: vec![],
                finish_reason: None,
                inference_timing: None,
                quality_gate_retries: 0,
                ..Default::default()
            },
            vec![],
        )
    }

    fn source(labels: usize, has_gap: bool) -> Arc<dyn ClassificationSource> {
        Arc::new(Fixed(SentinelEvidence {
            labels,
            tiers: 3,
            has_gap,
            details: serde_json::json!({"tiers": []}),
        }))
    }

    /// SENT-014: the check runs after the circuit check.
    #[spec("SENT-014")]
    #[test]
    fn the_check_runs_after_the_circuit_check() {
        assert!(SENTINEL_PRIORITY > CircuitCheck::new().priority());
    }

    /// SENT-014: the check contributes evidence, not a pass-or-fail verdict.
    #[spec("SENT-014")]
    #[tokio::test]
    async fn a_firing_label_contributes_evidence_rather_than_a_failure() {
        let check = SentinelCheck::new(source(2, false));

        let result = check.verify(&input("some completion")).await;

        assert!(
            !result.is_fail(),
            "the sentinel must never fail the pipeline"
        );
        let evidence = result.evidence().expect("evidence is attached");
        assert!(evidence.details.is_some());
    }

    /// SENT-014: a clean span passes and adds nothing.
    #[spec("SENT-014")]
    #[tokio::test]
    async fn a_clean_span_passes() {
        let check = SentinelCheck::new(source(0, false));

        assert!(check.verify(&input("nothing here")).await.is_pass());
    }

    /// SENT-013: a tier that could not answer is not a clean pass.
    #[spec("SENT-013")]
    #[tokio::test]
    async fn a_gap_is_reported_rather_than_passing() {
        let check = SentinelCheck::new(source(0, true));

        let result = check.verify(&input("nothing here")).await;

        assert!(
            !result.is_pass(),
            "an outage must not read as a clean result"
        );
    }

    /// SENT-014 edge case: a block is not softened by later evidence.
    #[spec("SENT-014")]
    #[tokio::test]
    async fn the_check_is_skipped_once_something_has_already_failed() {
        let check = SentinelCheck::new(source(2, false));

        assert!(check.skip_after_failure());
        // And existing checks are unaffected by the new trait method.
        assert!(!CircuitCheck::new().skip_after_failure());
    }

    /// SENT-014: existing circuit behaviour is unchanged when the sentinel is
    /// absent from the pipeline.
    #[spec("SENT-014")]
    #[tokio::test]
    async fn a_pipeline_without_the_sentinel_behaves_as_before() {
        let without = CriticPipeline::new().add_check(CircuitCheck::new());
        let with = CriticPipeline::new()
            .add_check(CircuitCheck::new())
            .add_check(SentinelCheck::new(source(0, false)));

        let a = without.verify(&input("plain")).await;
        let b = with.verify(&input("plain")).await;

        assert_eq!(a.passed, b.passed);
        assert_eq!(a.failures().len(), b.failures().len());
    }
    #[test]
    fn inspection_includes_reasoning() {
        let mut input = input("public answer");
        input.response.reasoning_content = Some("private reasoning".into());
        assert!(SentinelCheck::inspectable(&input).contains("private reasoning"));
    }
}
