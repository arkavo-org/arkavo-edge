//! Where the sentinel is plugged into the critic pipeline and the stream path
//! (SENT-007, SENT-009, SENT-014).
//!
//! Both seams are traits owned by the crates that need them — `arkavo-critic`
//! and `arkavo-llm` — because both sit underneath the classifier in the
//! dependency graph. This module is the one place above both, so it is where
//! the cascade actually meets them. A build without the `sentinel` feature
//! compiles neither adapter and behaves exactly as it did before.

use std::sync::{Arc, Mutex};

use arkavo_critic::{ClassificationSource, SentinelEvidence};
use arkavo_llm::{GateOutcome, ReleaseGate};
use arkavo_protocol::data_classification::SensitivityLevel;
use arkavo_sentinel::{Cascade, Holdback};

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
