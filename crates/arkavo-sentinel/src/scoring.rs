//! The sentinel tier and the executor that keeps it off the hot path
//! (SENT-002, SENT-011, SENT-013, SENT-016).
//!
//! The classifier itself is behind [`ScoringModel`] so this crate holds no
//! inference code: the GGUF-backed implementation lives where the model runtime
//! already is, and a node that runs only the reference tiers links none of it.
//!
//! Raw scores stop here (SENT-011). A `RawLabel` is what the model said; a
//! `LabelFinding` is what a threshold made of it. Nothing outside this module
//! sees the former, which is what keeps a score from riding out to a caller who
//! could use it to bisect the corpus.

use std::sync::Arc;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::thread;
use std::time::Instant;

use arkavo_protocol::classification_evidence::{
    ClassificationEvidence, Confidence, LabelFinding, TierReport,
};
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};

use crate::calibration::{CalibrationTable, ThresholdSource};
use crate::cascade::CascadeTier;

pub const SENTINEL_TIER_NAME: &str = "sentinel";

/// What the model said, before any threshold is applied.
#[derive(Debug, Clone, PartialEq)]
pub struct RawLabel {
    pub label: String,
    pub category: DataCategory,
    pub sensitivity: SensitivityLevel,
    /// The model's own score. Never leaves this crate.
    pub score: f32,
}

/// A classifier the sentinel tier can call.
///
/// Synchronous and blocking by design: the executor decides where the work
/// runs, and a model that scheduled its own asynchrony would take that decision
/// away from the thing responsible for the latency budget.
pub trait ScoringModel: Send + Sync {
    fn detector_version(&self) -> &str;
    fn taxonomy_version(&self) -> &str;
    /// Failures must remain inspection gaps, never successful empty findings.
    fn score(&self, text: &str) -> Result<Vec<RawLabel>, String>;
}

/// The sentinel as a cascade tier.
///
/// It never runs inline. `examine_until` reports a deferral whatever deadline
/// it is given, because a model that could be persuaded to run synchronously
/// for a small enough span is a model that can be made to run synchronously.
pub struct SentinelTier {
    model: Arc<dyn ScoringModel>,
    calibration: CalibrationTable,
    /// Set when the detector and taxonomy versions disagree, in which case the
    /// tier reports the mismatch instead of mapping labels it cannot vouch for.
    mismatch: Option<String>,
}

impl SentinelTier {
    /// SENT-015: the pairing is checked here, once, rather than per call. A
    /// mismatched pairing yields a tier that reports the gap forever, which is
    /// visible, instead of one that maps unknown labels onto known attributes.
    pub fn new(model: Arc<dyn ScoringModel>, calibration: CalibrationTable) -> Self {
        let mismatch = (!calibration.accepts_taxonomy(model.taxonomy_version())).then(|| {
            format!(
                "detector {} is calibrated against taxonomy {} but the model reports {}",
                calibration.detector_version,
                calibration.taxonomy_version,
                model.taxonomy_version()
            )
        });
        Self {
            model,
            calibration,
            mismatch,
        }
    }

    pub fn is_usable(&self) -> bool {
        self.mismatch.is_none()
    }

    fn version(&self) -> String {
        format!(
            "{}+{}",
            self.model.detector_version(),
            self.calibration.taxonomy_version
        )
    }

    /// Score a span and turn raw labels into findings.
    pub fn examine(&self, text: &str) -> TierReport {
        if let Some(reason) = &self.mismatch {
            return TierReport::unavailable(SENTINEL_TIER_NAME, self.version(), reason);
        }
        let raw = match self.model.score(text) {
            Ok(raw) => raw,
            Err(reason) => {
                return TierReport::unavailable(SENTINEL_TIER_NAME, self.version(), reason);
            }
        };
        let uncalibrated = self
            .calibration
            .uncalibrated(raw.iter().map(|label| label.label.as_str()));
        let mut findings: Vec<LabelFinding> = Vec::new();
        for label in raw {
            let (threshold, source) = self.calibration.threshold(&label.label);
            let confidence = Confidence::new(label.score);
            if confidence < threshold {
                continue;
            }
            // The signal names the label and how it was thresholded, never the
            // score: evidence is auditable without being an oracle readout.
            let calibration = match source {
                ThresholdSource::Calibrated => "calibrated",
                ThresholdSource::Uncalibrated => "uncalibrated",
            };
            findings.push(LabelFinding::new(
                label.category,
                label.sensitivity,
                confidence,
                format!("{} ({calibration})", label.label),
            ));
        }
        if !uncalibrated.is_empty() {
            tracing::warn!(
                labels = ?uncalibrated,
                detector = self.model.detector_version(),
                "sentinel emitted labels the calibration table does not cover"
            );
        }
        TierReport::matched(SENTINEL_TIER_NAME, self.version(), findings)
    }
}

impl CascadeTier for SentinelTier {
    fn name(&self) -> &str {
        SENTINEL_TIER_NAME
    }

    /// A mismatched pairing is not a tier that answers sometimes; it is one
    /// that cannot answer at all until the pairing is fixed.
    fn is_available(&self) -> bool {
        self.is_usable()
    }

    /// SENT-006, SENT-016: never inline, at any deadline.
    fn examine_until(&self, _text: &str, _deadline: Instant) -> TierReport {
        TierReport::unavailable(
            SENTINEL_TIER_NAME,
            self.version(),
            "sentinel scoring runs off the per-call path; deferred",
        )
    }

    fn examine_unbudgeted(&self, text: &str) -> TierReport {
        self.examine(text)
    }
}

/// A span waiting to be scored.
struct Job {
    text: String,
    reply: tokio::sync::oneshot::Sender<ClassificationEvidence>,
}

/// Why a span could not be queued.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScoringError {
    #[error("the scoring queue is full; the span stays held")]
    Saturated,
    #[error("the scoring executor has stopped")]
    Stopped,
}

/// Scoring on its own threads (SENT-016).
///
/// Its own OS threads rather than the caller's async executor: scoring is
/// CPU-bound, and running it on the runtime that serves tool calls would spend
/// the per-call budget it exists to protect.
pub struct ScoringExecutor {
    queue: SyncSender<Job>,
}

impl ScoringExecutor {
    /// Start `workers` threads over a queue of `capacity` spans.
    pub fn start(cascade: Arc<crate::cascade::Cascade>, capacity: usize, workers: usize) -> Self {
        let (queue, receiver) = sync_channel::<Job>(capacity);
        let receiver = Arc::new(std::sync::Mutex::new(receiver));
        for n in 0..workers.max(1) {
            let receiver = receiver.clone();
            let cascade = cascade.clone();
            // A worker that cannot be spawned is a smaller pool, not a failed
            // start: the caller holds content when the queue saturates, so
            // fewer workers costs latency rather than inspection.
            let spawned = thread::Builder::new()
                .name(format!("sentinel-score-{n}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let Ok(guard) = receiver.lock() else {
                                break;
                            };
                            guard.recv()
                        };
                        let Ok(job) = job else {
                            break;
                        };
                        let evidence = cascade.inspect_unbudgeted(&job.text);
                        // A dropped receiver means the consumer disconnected;
                        // the held content goes with it.
                        let _ = job.reply.send(evidence);
                    }
                });
            if let Err(e) = spawned {
                tracing::warn!(error = %e, "sentinel scoring worker could not start");
            }
        }
        Self { queue }
    }

    /// Queue a span. A full queue is refused rather than dropped: the caller's
    /// content stays held, which is the point of a bounded queue here
    /// (SENT-016 edge case).
    pub fn submit(
        &self,
        text: impl Into<String>,
    ) -> Result<tokio::sync::oneshot::Receiver<ClassificationEvidence>, ScoringError> {
        let (reply, receiver) = tokio::sync::oneshot::channel();
        let job = Job {
            text: text.into(),
            reply,
        };
        match self.queue.try_send(job) {
            Ok(()) => Ok(receiver),
            Err(TrySendError::Full(_)) => Err(ScoringError::Saturated),
            Err(TrySendError::Disconnected(_)) => Err(ScoringError::Stopped),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// A model whose answers the test controls, so the tier's own behaviour is
    /// what is being measured rather than a classifier's accuracy.
    struct Fixed {
        detector: String,
        taxonomy: String,
        labels: Vec<RawLabel>,
    }

    impl ScoringModel for Fixed {
        fn detector_version(&self) -> &str {
            &self.detector
        }

        fn taxonomy_version(&self) -> &str {
            &self.taxonomy
        }

        fn score(&self, _text: &str) -> Result<Vec<RawLabel>, String> {
            Ok(self.labels.clone())
        }
    }

    struct Failing;

    impl ScoringModel for Failing {
        fn detector_version(&self) -> &'static str {
            "sentinel-0.1"
        }
        fn taxonomy_version(&self) -> &'static str {
            "1.0.0"
        }
        fn score(&self, _text: &str) -> Result<Vec<RawLabel>, String> {
            Err("decode failed".into())
        }
    }

    #[spec("SENT-013")]
    #[test]
    fn model_failure_is_an_inspection_gap() {
        let tier = SentinelTier::new(Arc::new(Failing), calibration());
        let report = tier.examine("unclassified content");
        assert!(report.is_unavailable());
        assert!(format!("{report:?}").contains("decode failed"));
    }

    fn model(labels: Vec<RawLabel>) -> Arc<dyn ScoringModel> {
        Arc::new(Fixed {
            detector: "sentinel-0.1".into(),
            taxonomy: "1.0.0".into(),
            labels,
        })
    }

    fn raw(label: &str, sensitivity: SensitivityLevel, score: f32) -> RawLabel {
        RawLabel {
            label: label.into(),
            category: DataCategory::Credentials,
            sensitivity,
            score,
        }
    }

    fn calibration() -> CalibrationTable {
        CalibrationTable::new("sentinel-0.1", "1.0.0")
            .with_threshold("credentials", Confidence::new(0.8))
    }

    /// SENT-001: the tier returns labels and confidence, never allow or block.
    #[spec("SENT-001")]
    #[test]
    fn the_tier_returns_evidence_rather_than_a_verdict() {
        let tier = SentinelTier::new(
            model(vec![raw("credentials", SensitivityLevel::Restricted, 0.95)]),
            calibration(),
        );

        let report = tier.examine("some text");

        let finding = report.findings().first().expect("a finding");
        assert_eq!(finding.sensitivity, SensitivityLevel::Restricted);
        // Nothing in the report names a disposition.
        let rendered = format!("{report:?}");
        assert!(
            !rendered.contains("Allow") && !rendered.contains("Block"),
            "{rendered}"
        );
    }

    /// SENT-011: the model's own score never appears in the evidence signal,
    /// which is the text an auditor reads and a denial is rendered from.
    #[spec("SENT-011")]
    #[test]
    fn a_raw_score_never_reaches_the_evidence_signal() {
        let tier = SentinelTier::new(
            model(vec![raw(
                "credentials",
                SensitivityLevel::Restricted,
                0.9321,
            )]),
            calibration(),
        );

        let report = tier.examine("some text");

        let signal = &report.findings().first().expect("a finding").signal;
        assert!(!signal.contains("0.93"), "{signal}");
        assert!(signal.contains("credentials"), "{signal}");
    }

    /// SENT-004: a label below its calibrated threshold does not fire.
    #[spec("SENT-004")]
    #[test]
    fn a_label_below_its_threshold_does_not_fire() {
        let tier = SentinelTier::new(
            model(vec![raw("credentials", SensitivityLevel::Restricted, 0.5)]),
            calibration(),
        );

        assert!(tier.examine("some text").findings().is_empty());
    }

    /// SENT-004 edge case: a label the table omits fires anyway, marked as
    /// uncalibrated so the omission is visible in the evidence.
    #[spec("SENT-004")]
    #[test]
    fn an_uncalibrated_label_fires_and_says_so() {
        let tier = SentinelTier::new(
            model(vec![raw("healthcare", SensitivityLevel::Confidential, 0.1)]),
            calibration(),
        );

        let report = tier.examine("some text");

        let finding = report
            .findings()
            .first()
            .expect("an uncalibrated label fires");
        assert!(
            finding.signal.contains("uncalibrated"),
            "{}",
            finding.signal
        );
    }

    /// SENT-015: a detector paired with a taxonomy it was not calibrated
    /// against reports the mismatch instead of mapping labels it cannot vouch
    /// for, and names both versions.
    #[spec("SENT-015")]
    #[test]
    fn a_version_mismatch_makes_the_tier_report_a_gap() {
        let mismatched = Arc::new(Fixed {
            detector: "sentinel-0.1".into(),
            taxonomy: "2.0.0".into(),
            labels: vec![raw("credentials", SensitivityLevel::Restricted, 0.99)],
        });

        let tier = SentinelTier::new(mismatched, calibration());

        assert!(!tier.is_usable());
        let report = tier.examine("some text");
        assert!(report.is_unavailable());
        let rendered = format!("{report:?}");
        assert!(
            rendered.contains("1.0.0") && rendered.contains("2.0.0"),
            "{rendered}"
        );
    }

    /// SENT-006, SENT-016: the sentinel never runs inline, at any deadline.
    #[spec("SENT-016")]
    #[test]
    fn the_sentinel_defers_whatever_deadline_it_is_given() {
        let tier = SentinelTier::new(
            model(vec![raw("credentials", SensitivityLevel::Restricted, 0.99)]),
            calibration(),
        );

        let generous = tier.examine_until(
            "some text",
            Instant::now() + std::time::Duration::from_secs(60),
        );

        assert!(
            generous.is_unavailable(),
            "a generous deadline is still not an invitation"
        );
        assert!(!tier.examine_unbudgeted("some text").findings().is_empty());
    }

    /// SENT-016 edge case: a saturated queue holds the span rather than
    /// dropping the inspection.
    #[spec("SENT-016")]
    #[test]
    fn a_saturated_queue_refuses_rather_than_dropping_a_span() {
        // A cascade with no tiers, so workers finish instantly; the queue is
        // filled faster than it drains by submitting without awaiting.
        let cascade = Arc::new(crate::cascade::Cascade::new("1.0.0"));
        let executor = ScoringExecutor::start(cascade, 1, 0);
        drop(executor.submit("first"));

        let mut saturated = false;
        for _ in 0..64 {
            if executor.submit("another").err() == Some(ScoringError::Saturated) {
                saturated = true;
                break;
            }
        }

        assert!(saturated, "a bounded queue must eventually refuse");
    }

    #[test]
    fn a_submitted_span_comes_back_scored() {
        // Received without a runtime: the executor's threads are its own, which
        // is the property SENT-016 asks for.
        let cascade = Arc::new(crate::cascade::Cascade::new("1.0.0"));
        let executor = ScoringExecutor::start(cascade, 4, 1);

        let evidence = executor
            .submit("some text")
            .expect("queued")
            .blocking_recv()
            .expect("scored");

        assert_eq!(evidence.taxonomy_version, "1.0.0");
    }
}
