//! Where the sentinel is plugged into the critic pipeline and the stream path
//! (SENT-007, SENT-009, SENT-014).
//!
//! Both seams are traits owned by the crates that need them — `arkavo-critic`
//! and `arkavo-llm` — because both sit underneath the classifier in the
//! dependency graph. This module is the one place above both, so it is where
//! the cascade actually meets them. A build without the `sentinel` feature
//! compiles neither adapter and behaves exactly as it did before.

use std::sync::{Arc, Mutex};

use arkavo_critic::{ClassificationSource, SentinelCheck, SentinelEvidence};
use arkavo_fingerprint::IndexKey;
use arkavo_gguf_tdf::{Classification, PayloadKeyUnwrapper};
use arkavo_knowledge_pack::{LoadError, VerifiedPack, load_pack};
use arkavo_llm::{GateOutcome, ReleaseGate};
use arkavo_protocol::data_classification::SensitivityLevel;
use arkavo_sentinel::{CalibrationTable, Cascade, Holdback};

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
/// sharing this gate. Each completion gets its own gate in any case.
pub struct CascadeGate {
    cascade: Arc<Cascade>,
    holdback: Mutex<Holdback>,
}

impl CascadeGate {
    /// A gate for a model with the given classification ceiling.
    ///
    /// SENT-009: at Confidential or above the buffer streams nothing partial,
    /// and that comes from the ceiling rather than from anything a caller can
    /// pass here.
    pub fn new(cascade: Arc<Cascade>, ceiling: SensitivityLevel) -> Self {
        Self {
            cascade,
            holdback: Mutex::new(Holdback::for_ceiling(ceiling)),
        }
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
            let window = {
                let mut buffer = self.buffer();
                buffer.take_window()
            };
            let Some(window) = window else {
                return GateOutcome::Release(released);
            };
            // Inspection happens outside the lock: the window carries its own
            // overlap, so nothing else needs the buffer to read it.
            let evidence = self.cascade.inspect_unbudgeted(&window.inspect);
            // A gap is a reason to hold, not to release (SENT-013). Holding
            // here means blocking, because there is no later moment at which a
            // streamed token can be recalled.
            if evidence.findings().next().is_some() || evidence.has_gap() {
                self.buffer().block();
                return GateOutcome::Blocked;
            }
            released.push_str(&self.buffer().release());
        }
    }
}

impl ReleaseGate for CascadeGate {
    fn admit(&self, chunk: &str) -> GateOutcome {
        self.buffer().push(chunk);
        self.drain()
    }

    fn finish(&self) -> GateOutcome {
        self.buffer().finish();
        self.drain()
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
