//! The pattern detector as a cascade tier (SENT-002, SENT-006).
//!
//! `ClassificationInferencer` predates the cascade and returns
//! [`ClassifiedDatum`]s, which carry no confidence and no tier identity. This
//! adapter is where they acquire both, so evidence can say which tier produced
//! a label without the inferencer trait having to know a cascade exists.
//!
//! Pattern findings are reported as certain. That is not a claim the regex is
//! never wrong — it is the honest reading of a deterministic matcher: the
//! pattern either matched or it did not, and there is no calibrated
//! probability to report in between. Confidence that varies belongs to the
//! tiers that estimate rather than match.
//!
//! [`ClassifiedDatum`]: arkavo_protocol::data_classification::ClassifiedDatum

use std::sync::Arc;
use std::time::Instant;

use arkavo_protocol::ClassificationInferencer;
use arkavo_protocol::classification_evidence::{Confidence, LabelFinding, TierReport};

use crate::cascade::CascadeTier;

pub struct PatternTier {
    inferencer: Arc<dyn ClassificationInferencer>,
}

impl PatternTier {
    pub fn new(inferencer: Arc<dyn ClassificationInferencer>) -> Self {
        Self { inferencer }
    }

    fn report(&self, text: &str) -> TierReport {
        let findings: Vec<LabelFinding> = self
            .inferencer
            .infer(text)
            .into_iter()
            .map(|datum| {
                LabelFinding::new(
                    datum.category(),
                    datum.sensitivity(),
                    Confidence::CERTAIN,
                    format!("{:?}", datum.datum_type),
                )
                .at(datum.position.0, datum.position.1)
            })
            .collect();
        TierReport::matched(self.inferencer.name(), self.inferencer.version(), findings)
    }
}

impl CascadeTier for PatternTier {
    fn name(&self) -> &str {
        self.inferencer.name()
    }

    /// The pattern pass is one compiled expression over the span and cannot be
    /// stopped part-way, so it either runs or it reports that it did not. An
    /// already-expired deadline defers rather than overrunning the call.
    fn examine_until(&self, text: &str, deadline: Instant) -> TierReport {
        if Instant::now() >= deadline {
            return TierReport::unavailable(
                self.inferencer.name(),
                self.inferencer.version(),
                "no deadline left for the pattern pass; scoring deferred",
            );
        }
        self.report(text)
    }

    fn examine_unbudgeted(&self, text: &str) -> TierReport {
        self.report(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::RegexInferencer;
    use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
    use arkavo_test_macros::spec;

    fn tier() -> PatternTier {
        PatternTier::new(Arc::new(RegexInferencer::new()))
    }

    /// Built at run time so a literal that matches a secret pattern is not
    /// committed: a scanner that cries wolf on fixtures is one people ignore.
    fn fake_api_key() -> String {
        let prefix: String = ['s', 'k'].iter().collect();
        let body: String = (0..24)
            .map(|i| char::from(b'a' + ((i * 7 + 3) % 26) as u8))
            .collect();
        format!("{prefix}-{body}")
    }

    /// SENT-002: evidence carries the detector version and localizes the span.
    #[spec("SENT-002")]
    #[test]
    fn a_pattern_match_carries_its_detector_version_and_span() {
        let report = tier().examine_unbudgeted(&format!("the key is {} ok", fake_api_key()));

        let finding = report.findings().first().expect("a credential");
        assert_eq!(finding.category, DataCategory::Credentials);
        assert_eq!(finding.sensitivity, SensitivityLevel::Restricted);
        assert!(finding.span.is_some());
        assert!(!report.version.is_empty());
    }

    /// SENT-002 edge case: a clean pass is recorded as consulted with no match.
    #[spec("SENT-002")]
    #[test]
    fn a_clean_span_is_recorded_as_consulted() {
        let report = tier().examine_unbudgeted("the quick brown fox jumps over the lazy dog");

        assert!(!report.is_unavailable());
        assert!(report.findings().is_empty());
    }

    /// SENT-011: the evidence signal names the pattern, never the text it
    /// matched. A signal carrying the secret would put it in every audit line.
    #[spec("SENT-011")]
    #[test]
    fn the_signal_names_the_pattern_and_not_the_matched_text() {
        let key = fake_api_key();

        let report = tier().examine_unbudgeted(&format!("the key is {key} ok"));

        let signal = &report.findings().first().expect("a credential").signal;
        assert!(!signal.contains(&key), "{signal}");
    }

    /// An expired deadline defers rather than overrunning the call.
    #[spec("SENT-016")]
    #[test]
    fn an_expired_deadline_defers_the_pattern_pass() {
        let report = tier().examine_until("anything", Instant::now());

        assert!(report.is_unavailable());
    }
}
