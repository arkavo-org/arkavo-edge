//! Where the sentinel is plugged into the critic pipeline and the stream path
//! (SENT-007, SENT-009, SENT-014).
//!
//! Both seams are traits owned by the crates that need them — `arkavo-critic`
//! and `arkavo-llm` — because both sit underneath the classifier in the
//! dependency graph. This module is the one place above both, so it is where
//! the cascade actually meets them. A build without the `sentinel` feature
//! compiles neither adapter and behaves exactly as it did before.

use std::path::Path;
use std::sync::{Arc, Mutex};

use arkavo_critic::{ClassificationSource, SentinelCheck, SentinelEvidence};
use arkavo_fingerprint::IndexKey;
use arkavo_gguf_tdf::{Classification, PayloadKeyUnwrapper};
use arkavo_knowledge_pack::{LoadError, VerifiedPack, load_pack};
use arkavo_llm::{GateOutcome, ReleaseGate};
use arkavo_protocol::RegexInferencer;
use arkavo_protocol::data_classification::SensitivityLevel;
use arkavo_sentinel::{
    CalibrationTable, Cascade, Holdback, HoldbackState, PatternTier, SentinelTier,
};

use crate::sentinel_scorer::LlamaScoringModel;

/// Bytes of generated text the distilled sentinel inspects at once.
///
/// The holdback's own default is sized for the reference tier's word shingles,
/// which recognize a span a few words long. The distilled detector is not that:
/// it was fine-tuned on page-sized passages, and a 256-byte fragment is roughly
/// eighty tokens — a length it never saw during training, so its confidence
/// there describes nothing the calibration measured. A kilobyte is a few
/// hundred tokens, inside the distribution the thresholds were fitted on, and
/// still short enough that a long answer arrives in pieces.
pub const SENTINEL_WINDOW_BYTES: usize = 1024;

/// The cascade as the critic pipeline sees it.
pub struct CascadeSource {
    cascade: Arc<Cascade>,
}

impl CascadeSource {
    pub fn new(cascade: Arc<Cascade>) -> Self {
        Self { cascade }
    }
}

impl ClassificationSource for CascadeSource {
    fn inspect(&self, text: &str) -> SentinelEvidence {
        // Unbudgeted: the critic pipeline is not the per-tool-call path, and
        // the cascade's deadline exists to protect that path rather than this.
        let evidence = self.cascade.inspect_unbudgeted(text);
        SentinelEvidence {
            labels: evidence.findings().count(),
            tiers: evidence.tiers.len(),
            has_gap: evidence.has_gap(),
            details: serde_json::to_value(&evidence).unwrap_or(serde_json::Value::Null),
        }
    }
}

/// A holdback buffer driven by the cascade, as the stream path sees it.
///
/// The lock is held only across a buffer operation, never across inspection, so
/// a slow tier delays the completion it is inspecting rather than every stream
/// sharing this gate.
///
/// One gate serves a session rather than a single completion, because the
/// router holds it for as long as it holds the session. The buffer is therefore
/// replaced after each completion that finishes cleanly: without that, the
/// overlap carried into the next completion's first window would be the *end of
/// the previous answer*, and the gate would be judging text no model ever
/// produced in one span. A buffer that blocked is not replaced — a blocked
/// session stays blocked, which is the safe direction.
pub struct CascadeGate {
    cascade: Arc<Cascade>,
    ceiling: SensitivityLevel,
    window_bytes: usize,
    overlap_bytes: usize,
    holdback: Mutex<Holdback>,
}

impl CascadeGate {
    /// A gate for a model with the given classification ceiling, sized by the
    /// holdback's own defaults.
    ///
    /// SENT-009: at Confidential or above the buffer streams nothing partial,
    /// and that comes from the ceiling rather than from anything a caller can
    /// pass here.
    pub fn new(cascade: Arc<Cascade>, ceiling: SensitivityLevel) -> Self {
        Self::with_holdback(
            cascade,
            ceiling,
            arkavo_sentinel::DEFAULT_WINDOW_BYTES,
            arkavo_sentinel::DEFAULT_OVERLAP_BYTES,
        )
    }

    /// A gate whose inspection window is sized to what its tiers were trained
    /// on.
    ///
    /// The defaults are sized for the reference tier's shingles. A learned tier
    /// judged spans of a particular length during training, and handing it a
    /// fragment a fraction of that size asks it a question it was never
    /// calibrated to answer — so the caller that knows the detector sets the
    /// window. The ceiling still wins: no window a caller passes reintroduces
    /// partial streaming above Confidential.
    pub fn with_holdback(
        cascade: Arc<Cascade>,
        ceiling: SensitivityLevel,
        window_bytes: usize,
        overlap_bytes: usize,
    ) -> Self {
        Self {
            cascade,
            ceiling,
            window_bytes,
            overlap_bytes,
            holdback: Mutex::new(Self::fresh_buffer(ceiling, window_bytes, overlap_bytes)),
        }
    }

    fn fresh_buffer(
        ceiling: SensitivityLevel,
        window_bytes: usize,
        overlap_bytes: usize,
    ) -> Holdback {
        if ceiling >= SensitivityLevel::Confidential {
            return Holdback::for_ceiling(ceiling);
        }
        Holdback::new(window_bytes, overlap_bytes)
    }

    fn buffer(&self) -> std::sync::MutexGuard<'_, Holdback> {
        // A panicking inspection must not release what it was inspecting, so a
        // poisoned buffer is recovered rather than propagated: its contents are
        // still held text, and holding is the safe state.
        self.holdback.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Inspect every ready window, releasing what clears and blocking on the
    /// first that fires.
    fn drain(&self) -> GateOutcome {
        let mut released = String::new();
        loop {
            let (window, state) = {
                let mut buffer = self.buffer();
                (buffer.take_window(), buffer.state())
            };
            let Some(window) = window else {
                // A buffer with no window ready is either still accumulating or
                // finished with the label that stopped it. The second is not a
                // release of nothing: it is the refusal, still standing. Saying
                // `Release` here would hand the caller an empty completion and,
                // worse, let `finish` mistake a blocked buffer for a clean one
                // and reset it — unblocking the session the block was meant to
                // end.
                if state == HoldbackState::Blocked {
                    return GateOutcome::Blocked;
                }
                return GateOutcome::Release(released);
            };
            // Inspection happens outside the lock: the window carries its own
            // overlap, so nothing else needs the buffer to read it.
            let evidence = self.cascade.inspect_unbudgeted(&window.inspect);
            // A gap is a reason to hold, not to release (SENT-013). Holding
            // here means blocking, because there is no later moment at which a
            // streamed token can be recalled.
            //
            // A Public finding is not one. The tiers report evidence and this
            // is where it becomes a decision: a pattern tier only speaks when it
            // recognizes something, but a learned tier labels every span it is
            // shown, and its verdict on ordinary text is a finding that says
            // "public" at full confidence. Withholding on that withholds
            // everything. The finding still reaches the critic pipeline through
            // `CascadeSource`, where a classification of Public is worth
            // recording; it is only here, where the question is whether to
            // withhold, that it means nothing.
            let withhold = evidence
                .findings()
                .any(|finding| finding.sensitivity > SensitivityLevel::Public);
            if withhold || evidence.has_gap() {
                self.buffer().block();
                return GateOutcome::Blocked;
            }
            released.push_str(&self.buffer().release());
        }
    }
}

/// The classification ceiling named on the command line.
///
/// Restricted is deliberately not spellable here: it is a pack's property, not
/// a flag's, and a caller who could name it could claim a ceiling for a model
/// whose provenance nothing checked.
pub fn parse_ceiling(name: &str) -> Result<SensitivityLevel, String> {
    match name {
        "public" => Ok(SensitivityLevel::Public),
        "internal" => Ok(SensitivityLevel::Internal),
        "confidential" => Ok(SensitivityLevel::Confidential),
        other => Err(format!(
            "unknown ceiling '{other}': expected public, internal or confidential"
        )),
    }
}

fn ceiling_name(ceiling: SensitivityLevel) -> &'static str {
    match ceiling {
        SensitivityLevel::Public => "public",
        SensitivityLevel::Internal => "internal",
        SensitivityLevel::Confidential => "confidential",
        SensitivityLevel::Restricted => "restricted",
    }
}

/// Build the gate a command arms from a detector GGUF and its calibration.
///
/// The pattern tier runs first because it is the cheap one and the cascade's
/// order is its contract; the distilled detector runs second, against the
/// thresholds the same file names. Returns the gate and the one line that says
/// what was armed, so the caller decides where that line goes.
///
/// This is the unsigned path: a pack (see [`SentinelRuntime::from_pack`])
/// carries its thresholds under a signature, and these two files carry nothing
/// but themselves. It exists so the detector can be exercised end to end before
/// there is a pack to seal it into.
pub fn armed_gate(
    detector: &Path,
    calibration: &Path,
    ceiling: SensitivityLevel,
) -> Result<(CascadeGate, String), String> {
    let json = std::fs::read_to_string(calibration)
        .map_err(|e| format!("cannot read calibration {}: {e}", calibration.display()))?;
    // A table that will not parse is not an empty table. An empty one calibrates
    // no label, an uncalibrated label fires, and the gate would then block every
    // completion while reporting itself armed — a policy nobody wrote.
    let table = CalibrationTable::from_json(&json)
        .map_err(|e| format!("cannot parse calibration {}: {e}", calibration.display()))?;

    // The versions travel from the table into the model so that the tier's
    // pairing check compares the detector against the table that calibrated it.
    let detector_version = table.detector_version.clone();
    let model = LlamaScoringModel::load(detector, &detector_version, &table.taxonomy_version)?;

    let cascade = Arc::new(
        Cascade::new(table.taxonomy_version.clone())
            .with_tier(Arc::new(PatternTier::new(Arc::new(RegexInferencer::new()))))
            .with_tier(Arc::new(SentinelTier::new(Arc::new(model), table))),
    );
    let armed = format!(
        "[sentinel] gate armed: tiers {}; ceiling {}; detector {detector_version}",
        cascade.tier_names().join(", "),
        ceiling_name(ceiling)
    );
    let gate = CascadeGate::with_holdback(
        cascade,
        ceiling,
        SENTINEL_WINDOW_BYTES,
        SENTINEL_WINDOW_BYTES / 4,
    );
    Ok((gate, armed))
}

impl ReleaseGate for CascadeGate {
    fn admit(&self, chunk: &str) -> GateOutcome {
        self.buffer().push(chunk);
        self.drain()
    }

    fn finish(&self) -> GateOutcome {
        self.buffer().finish();
        let outcome = self.drain();
        if matches!(outcome, GateOutcome::Release(_)) {
            // The completion is over and cleared. The next one starts against
            // an empty buffer rather than inheriting this one's overlap and its
            // spent `finished` flag, which would make the next first window
            // both shorter than configured and prefixed by another answer.
            *self.buffer() =
                Self::fresh_buffer(self.ceiling, self.window_bytes, self.overlap_bytes);
        }
        outcome
    }

    fn discard(&self) {
        self.buffer().discard();
    }
}

/// Everything a session needs to enforce classification, provisioned from one
/// verified pack.
///
/// This is the seam Phase 4 left open. The cascade, the gate and the critic
/// check were all constructible then and nothing constructed them, because the
/// indices and thresholds they need come from a pack — and there was no pack.
/// The ordering that matters is inside `load_pack`: the manifest's signature
/// and every present component's digest have been checked before any of this
/// exists, so none of it can be built from content nobody vouched for.
pub struct SentinelRuntime {
    cascade: Arc<Cascade>,
    /// Calibrated thresholds, from the signed manifest rather than from
    /// anything an operator can edit locally (SENT-004).
    pub calibration: CalibrationTable,
    /// The ceiling anything served under this pack carries.
    pub ceiling: SensitivityLevel,
    /// What this node holds, for the audit record.
    pub inventory: String,
}

impl SentinelRuntime {
    /// Provision from a pack whose signature and digests already verified.
    pub fn from_pack(
        pack: &VerifiedPack,
        index_key: Option<&Arc<IndexKey>>,
        unwrapper: &dyn PayloadKeyUnwrapper,
    ) -> Result<Self, LoadError> {
        let loaded = load_pack(pack, index_key, unwrapper)?;
        Ok(Self {
            cascade: loaded.cascade,
            calibration: loaded.calibration,
            ceiling: sensitivity_of(loaded.ceiling),
            inventory: loaded.inventory,
        })
    }

    /// The critic-pipeline check this pack's cascade backs.
    pub fn check(&self) -> SentinelCheck {
        SentinelCheck::new(Arc::new(CascadeSource::new(self.cascade.clone())))
    }

    /// A release gate for one completion.
    ///
    /// One gate per completion, never shared: the holdback buffer holds *that*
    /// completion's text, and a shared one would let two streams block each
    /// other or, worse, release each other's windows. The ceiling comes from
    /// the pack rather than from the caller (SENT-009).
    pub fn gate(&self) -> CascadeGate {
        CascadeGate::new(self.cascade.clone(), self.ceiling)
    }

    pub fn cascade(&self) -> Arc<Cascade> {
        self.cascade.clone()
    }
}

/// Map the pack's classification vocabulary onto the taint vocabulary.
///
/// Two enums rather than one because the format crate sits underneath the crate
/// that owns classification and cannot depend on it. This is the one place the
/// two meet, so it is the one place the mapping can drift.
fn sensitivity_of(classification: Classification) -> SensitivityLevel {
    match classification {
        Classification::Public => SensitivityLevel::Public,
        Classification::Internal => SensitivityLevel::Internal,
        Classification::Confidential => SensitivityLevel::Confidential,
        Classification::Restricted => SensitivityLevel::Restricted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::classification_evidence::{Confidence, LabelFinding, TierReport};
    use arkavo_protocol::data_classification::DataCategory;
    use arkavo_sentinel::{CascadeTier, RawLabel, ScoringModel};
    use arkavo_test_macros::spec;
    use std::time::Instant;

    /// A tier that finds nothing and remembers every span it was shown, so a
    /// test can assert on what the gate offered for inspection rather than only
    /// on what it released.
    struct Recording {
        seen: Mutex<Vec<String>>,
    }

    impl Recording {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                seen: Mutex::new(Vec::new()),
            })
        }

        fn windows(&self) -> Vec<String> {
            self.seen.lock().expect("lock").clone()
        }
    }

    impl CascadeTier for Recording {
        fn name(&self) -> &str {
            "recording"
        }

        fn examine_until(&self, text: &str, _deadline: Instant) -> TierReport {
            self.examine_unbudgeted(text)
        }

        fn examine_unbudgeted(&self, text: &str) -> TierReport {
            self.seen.lock().expect("lock").push(text.to_string());
            TierReport::matched("recording", "1.0.0", Vec::new())
        }
    }

    fn gate_with(tier: Arc<dyn CascadeTier>, ceiling: SensitivityLevel) -> CascadeGate {
        CascadeGate::with_holdback(
            Arc::new(Cascade::new("1.0.0").with_tier(tier)),
            ceiling,
            SENTINEL_WINDOW_BYTES,
            SENTINEL_WINDOW_BYTES / 4,
        )
    }

    /// SENT-007: the window the detector is asked about is the size it was
    /// configured with, not the holdback's default. A detector trained on
    /// page-sized spans that is handed a sentence is being asked a different
    /// question than the one its thresholds were fitted to.
    #[spec("SENT-007")]
    #[test]
    fn the_first_inspection_window_is_the_configured_size() {
        let tier = Recording::new();
        let gate = gate_with(tier.clone(), SensitivityLevel::Internal);

        // One byte short of a window: nothing has been inspected and nothing
        // released.
        let short = gate.admit(&"a".repeat(SENTINEL_WINDOW_BYTES - 1));
        assert_eq!(short, GateOutcome::Release(String::new()));
        assert!(tier.windows().is_empty(), "{:?}", tier.windows());

        // Completing the window releases exactly one window's worth; the 75
        // bytes past it stay held until another window fills or the completion
        // ends.
        let rest = gate.admit(&"a".repeat(101));
        assert_eq!(
            rest,
            GateOutcome::Release("a".repeat(SENTINEL_WINDOW_BYTES))
        );
        assert_eq!(tier.windows().len(), 1);
        assert_eq!(tier.windows()[0].len(), SENTINEL_WINDOW_BYTES);

        // And the tail is inspected rather than flushed.
        assert_eq!(gate.finish(), GateOutcome::Release("a".repeat(100)));
    }

    /// SENT-009: the ceiling still decides. A window size the caller passes
    /// cannot reintroduce partial streaming above Confidential.
    #[spec("SENT-009")]
    #[test]
    fn a_confidential_ceiling_releases_nothing_before_the_completion_ends() {
        let gate = gate_with(Recording::new(), SensitivityLevel::Confidential);

        let partial = gate.admit(&"a".repeat(SENTINEL_WINDOW_BYTES * 3));

        assert_eq!(partial, GateOutcome::Release(String::new()));
        assert_eq!(
            gate.finish(),
            GateOutcome::Release("a".repeat(SENTINEL_WINDOW_BYTES * 3))
        );
    }

    /// One gate serves a session, so a completion that cleared must not leave
    /// its tail — or its spent finished flag — in the buffer the next one is
    /// judged in.
    #[spec("SENT-007")]
    #[test]
    fn a_second_completion_starts_from_an_empty_buffer() {
        let tier = Recording::new();
        let gate = gate_with(tier.clone(), SensitivityLevel::Internal);

        assert_eq!(
            gate.admit("first answer"),
            GateOutcome::Release(String::new())
        );
        assert_eq!(
            gate.finish(),
            GateOutcome::Release("first answer".to_string())
        );

        // Still short of a window, so still held: a buffer that kept the first
        // completion's `finished` flag would release this immediately.
        assert_eq!(
            gate.admit("second answer"),
            GateOutcome::Release(String::new())
        );
        assert_eq!(
            gate.finish(),
            GateOutcome::Release("second answer".to_string())
        );

        let windows = tier.windows();
        assert_eq!(windows, vec!["first answer", "second answer"]);
    }

    /// A detector that says the same thing about every span, so a test can fix
    /// what the tier is told and assert on what the gate does with it.
    struct FixedScores(Vec<RawLabel>);

    impl ScoringModel for FixedScores {
        fn detector_version(&self) -> &str {
            "fixed"
        }

        fn taxonomy_version(&self) -> &str {
            "1.0.0"
        }

        fn score(&self, _text: &str) -> Result<Vec<RawLabel>, String> {
            Ok(self.0.clone())
        }
    }

    fn label(name: &str, sensitivity: SensitivityLevel, score: f32) -> RawLabel {
        RawLabel {
            label: name.to_string(),
            category: DataCategory::Internal,
            sensitivity,
            score,
        }
    }

    /// The thresholds the distilled detector is calibrated at: the confidential
    /// label is tuned to a false-positive rate and the other two sit at the
    /// argmax boundary.
    fn calibrated() -> CalibrationTable {
        CalibrationTable::new("fixed", "1.0.0")
            .with_threshold("public", Confidence::new(0.5))
            .with_threshold("internal", Confidence::new(0.5))
            .with_threshold("confidential", Confidence::new(0.003))
    }

    fn sentinel_gate(labels: Vec<RawLabel>) -> CascadeGate {
        let tier = SentinelTier::new(Arc::new(FixedScores(labels)), calibrated());
        CascadeGate::with_holdback(
            Arc::new(Cascade::new("1.0.0").with_tier(Arc::new(tier))),
            SensitivityLevel::Internal,
            SENTINEL_WINDOW_BYTES,
            SENTINEL_WINDOW_BYTES / 4,
        )
    }

    /// Regression: a learned tier labels *every* span, so ordinary text comes
    /// back carrying a `public` finding above its threshold. Treating that as a
    /// reason to withhold withheld every completion — the gate blocked "Say
    /// hello." as readily as a leak.
    #[spec("SENT-007")]
    #[test]
    fn a_public_label_is_evidence_rather_than_a_reason_to_withhold() {
        let gate = sentinel_gate(vec![
            label("public", SensitivityLevel::Public, 1.0),
            label("internal", SensitivityLevel::Internal, 1.0e-9),
            label("confidential", SensitivityLevel::Confidential, 6.8e-7),
        ]);

        assert_eq!(gate.admit("hello"), GateOutcome::Release(String::new()));
        assert_eq!(gate.finish(), GateOutcome::Release("hello".to_string()));
    }

    /// And the tier still stops what it is for. The threshold is the tuned one,
    /// so a score three orders of magnitude below certainty still fires.
    #[spec("SENT-007")]
    #[test]
    fn a_confidential_label_above_its_threshold_withholds_the_completion() {
        let gate = sentinel_gate(vec![
            label("public", SensitivityLevel::Public, 0.0),
            label("internal", SensitivityLevel::Internal, 0.0),
            label("confidential", SensitivityLevel::Confidential, 0.004),
        ]);

        gate.admit("the northwind acquisition closes in the third quarter");

        assert_eq!(gate.finish(), GateOutcome::Blocked);
    }

    /// A tier that speaks only when it recognizes something, the way a pattern
    /// or reference tier does — needed to tell one blocked completion from the
    /// clean ones around it.
    struct OnNeedle(&'static str);

    impl CascadeTier for OnNeedle {
        fn name(&self) -> &str {
            "needle"
        }

        fn examine_until(&self, text: &str, _deadline: Instant) -> TierReport {
            self.examine_unbudgeted(text)
        }

        fn examine_unbudgeted(&self, text: &str) -> TierReport {
            let findings = if text.contains(self.0) {
                vec![LabelFinding::new(
                    DataCategory::Internal,
                    SensitivityLevel::Confidential,
                    Confidence::new(1.0),
                    "needle",
                )]
            } else {
                Vec::new()
            };
            TierReport::matched("needle", "1.0.0", findings)
        }
    }

    /// Regression: the buffer is replaced after a completion that *cleared*, and
    /// a blocked buffer must not look like one. It reports no window, and
    /// reading that as an empty release both handed the next completion back
    /// blank and reset the buffer underneath it — so the message after a block
    /// came back empty and every message after that streamed ungated.
    #[spec("SENT-007")]
    #[test]
    fn a_blocked_session_stays_blocked() {
        let gate = gate_with(Arc::new(OnNeedle("CANARY")), SensitivityLevel::Internal);

        gate.admit("a completion carrying CANARY");
        assert_eq!(gate.finish(), GateOutcome::Blocked);

        // The next completion is refused rather than returned empty, and the
        // one after it too.
        assert_eq!(gate.admit("entirely clean text"), GateOutcome::Blocked);
        assert_eq!(gate.finish(), GateOutcome::Blocked);
        assert_eq!(gate.admit("still entirely clean"), GateOutcome::Blocked);
        assert_eq!(gate.finish(), GateOutcome::Blocked);
    }

    #[test]
    fn the_ceiling_flag_names_only_the_levels_a_caller_may_claim() {
        assert_eq!(parse_ceiling("public"), Ok(SensitivityLevel::Public));
        assert_eq!(parse_ceiling("internal"), Ok(SensitivityLevel::Internal));
        assert_eq!(
            parse_ceiling("confidential"),
            Ok(SensitivityLevel::Confidential)
        );
        assert!(parse_ceiling("restricted").is_err());
    }

    /// A calibration file that will not parse arms nothing: an empty table
    /// calibrates no label, and an uncalibrated label fires.
    #[spec("SENT-004")]
    #[test]
    fn an_unparsable_calibration_is_refused_rather_than_emptied() {
        let dir = std::env::temp_dir().join("arkavo-sentinel-wiring-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("not-a-calibration.json");
        std::fs::write(&path, b"{ not json").expect("write");

        let Err(error) = armed_gate(&dir.join("absent.gguf"), &path, SensitivityLevel::Internal)
        else {
            panic!("a malformed table must not arm a gate");
        };

        assert!(error.contains("cannot parse calibration"), "{error}");
    }

    #[test]
    fn the_two_classification_scales_stay_aligned() {
        // Ordering is what the high-water merge relies on, so the mapping has
        // to preserve it as well as the names.
        assert!(
            sensitivity_of(Classification::Restricted)
                > sensitivity_of(Classification::Confidential)
        );
        assert!(
            sensitivity_of(Classification::Confidential) > sensitivity_of(Classification::Internal)
        );
        assert!(sensitivity_of(Classification::Internal) > sensitivity_of(Classification::Public));
    }
}
