//! Surprise detection trait and implementations
//!
//! Three built-in detectors for the three-level anomaly classification:
//! - Level 1: `HardFailureDetector` - Runtime errors
//! - Level 2: `BoundaryDetector` - Invariant violations via SBE
//! - Level 3: `DriftDetector` - Statistical distribution anomalies

use std::collections::HashMap;
use std::sync::Arc;

use arkavo_sbe::InvariantLayer;
use torg_core::EvalError;

use crate::accumulator::EmaAccumulator;
use crate::evidence::{AnomalyEvidence, hash_inputs};

/// O(1) or O(N) complexity surprise classification
///
/// Implementations must complete inspection in <4µs to meet the <5µs total budget.
pub trait SurpriseDetector: Send + Sync {
    /// Inspect a transaction for anomalies
    ///
    /// # Arguments
    /// - `inputs`: The input values used for evaluation
    /// - `outputs`: The output values from evaluation (empty if error)
    /// - `error`: The evaluation error, if any
    ///
    /// # Returns
    /// Evidence if an anomaly is detected, None otherwise
    fn inspect(
        &mut self,
        inputs: &HashMap<u16, bool>,
        outputs: &HashMap<u16, bool>,
        error: Option<&EvalError>,
    ) -> Option<AnomalyEvidence>;

    /// Update baseline statistics (called periodically, not per-transaction)
    fn update_baseline(&mut self) {}
}

/// Level 1: Detects hard failures (evaluation errors)
///
/// Overhead: ~0.1µs (single match on Option)
#[derive(Debug, Clone, Default)]
pub struct HardFailureDetector;

impl HardFailureDetector {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl SurpriseDetector for HardFailureDetector {
    #[inline]
    fn inspect(
        &mut self,
        inputs: &HashMap<u16, bool>,
        _outputs: &HashMap<u16, bool>,
        error: Option<&EvalError>,
    ) -> Option<AnomalyEvidence> {
        error.map(|e| AnomalyEvidence::hard_failure(e.clone(), hash_inputs(inputs)))
    }
}

/// Level 2: Detects boundary violations using SBE InvariantLayer
///
/// Overhead: ~1.0µs (HashMap iteration + invariant check)
#[derive(Debug, Clone)]
pub struct BoundaryDetector {
    invariants: Arc<InvariantLayer>,
}

impl BoundaryDetector {
    /// Create a new boundary detector with the given invariant layer
    #[must_use]
    pub fn new(invariants: Arc<InvariantLayer>) -> Self {
        Self { invariants }
    }
}

impl SurpriseDetector for BoundaryDetector {
    #[inline]
    fn inspect(
        &mut self,
        inputs: &HashMap<u16, bool>,
        _outputs: &HashMap<u16, bool>,
        _error: Option<&EvalError>,
    ) -> Option<AnomalyEvidence> {
        // Check if inputs violate any invariant constraints
        self.invariants.check_violation(inputs).map(|violation| {
            AnomalyEvidence::boundary_violation(
                violation.contract_id,
                format!("Invariant violated at {:?}", violation.detected_at),
                hash_inputs(inputs),
            )
        })
    }
}

/// Level 3: Detects statistical drift using EMA
///
/// Overhead: ~0.5µs (array lookup + comparison)
#[derive(Debug, Clone)]
pub struct DriftDetector {
    accumulator: EmaAccumulator,
}

impl Default for DriftDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl DriftDetector {
    /// Create a new drift detector with default settings
    #[must_use]
    pub fn new() -> Self {
        Self {
            accumulator: EmaAccumulator::new(),
        }
    }

    /// Create with custom accumulator settings
    #[must_use]
    pub fn with_config(alpha: f32, z_threshold: f32, min_samples: u32) -> Self {
        Self {
            accumulator: EmaAccumulator::with_config(alpha, z_threshold, min_samples),
        }
    }

    /// Get mutable access to the accumulator for updates
    pub fn accumulator_mut(&mut self) -> &mut EmaAccumulator {
        &mut self.accumulator
    }
}

impl SurpriseDetector for DriftDetector {
    #[inline]
    fn inspect(
        &mut self,
        inputs: &HashMap<u16, bool>,
        outputs: &HashMap<u16, bool>,
        _error: Option<&EvalError>,
    ) -> Option<AnomalyEvidence> {
        let input_hash = hash_inputs(inputs);

        // Check each output for drift
        for (&output_id, &value) in outputs {
            if let Some(z_score) = self.accumulator.update(output_id, value) {
                let baseline = self.accumulator.get_ema(output_id).unwrap_or(0.5);
                let current = if value { 1.0 } else { 0.0 };

                return Some(AnomalyEvidence::drift(
                    output_id, baseline, current, z_score, input_hash,
                ));
            }
        }

        None
    }

    fn update_baseline(&mut self) {
        // Could implement periodic baseline recalibration here
    }
}

/// Composite detector that chains multiple detectors
///
/// Returns evidence from the first detector that triggers.
#[derive(Default)]
pub struct CompositeDetector {
    detectors: Vec<Box<dyn SurpriseDetector>>,
}

impl CompositeDetector {
    /// Create a new empty composite detector
    #[must_use]
    pub fn new() -> Self {
        Self {
            detectors: Vec::new(),
        }
    }

    /// Add a detector to the chain
    pub fn add(&mut self, detector: Box<dyn SurpriseDetector>) {
        self.detectors.push(detector);
    }

    /// Create with the standard three-level detection chain
    #[must_use]
    pub fn standard(invariants: Arc<InvariantLayer>) -> Self {
        let mut composite = Self::new();
        composite.add(Box::new(HardFailureDetector::new()));
        composite.add(Box::new(BoundaryDetector::new(invariants)));
        composite.add(Box::new(DriftDetector::new()));
        composite
    }
}

impl SurpriseDetector for CompositeDetector {
    #[inline]
    fn inspect(
        &mut self,
        inputs: &HashMap<u16, bool>,
        outputs: &HashMap<u16, bool>,
        error: Option<&EvalError>,
    ) -> Option<AnomalyEvidence> {
        for detector in &mut self.detectors {
            if let Some(evidence) = detector.inspect(inputs, outputs, error) {
                return Some(evidence);
            }
        }
        None
    }

    fn update_baseline(&mut self) {
        for detector in &mut self.detectors {
            detector.update_baseline();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hard_failure_detector() {
        let mut detector = HardFailureDetector::new();
        let inputs = HashMap::new();
        let outputs = HashMap::new();

        // No error - no detection
        let result = detector.inspect(&inputs, &outputs, None);
        assert!(result.is_none());

        // With error - detection
        let error = EvalError::MissingInput(0);
        let result = detector.inspect(&inputs, &outputs, Some(&error));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().level,
            crate::evidence::AnomalyLevel::HardFailure
        );
    }

    #[test]
    fn test_boundary_detector_no_violation() {
        let invariants = Arc::new(InvariantLayer::new());
        let mut detector = BoundaryDetector::new(invariants);

        let inputs = HashMap::new();
        let outputs = HashMap::new();

        let result = detector.inspect(&inputs, &outputs, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_drift_detector() {
        // Test that drift detector properly integrates with accumulator
        let mut detector = DriftDetector::new();

        let inputs = HashMap::new();
        let mut outputs = HashMap::new();
        outputs.insert(0, true);

        // No drift during warmup (min_samples not reached)
        for _ in 0..50 {
            let result = detector.inspect(&inputs, &outputs, None);
            assert!(result.is_none(), "Should not detect drift during warmup");
        }

        // Verify accumulator is warming up (not yet at 100 samples)
        assert!(!detector.accumulator.is_warmed_up(0));

        // Continue warmup to 100+ samples
        for _ in 0..60 {
            detector.inspect(&inputs, &outputs, None);
        }

        // Should be warmed up now
        assert!(detector.accumulator.is_warmed_up(0));

        // EMA should be close to 1.0 after many true values
        let ema = detector.accumulator.get_ema(0).unwrap();
        assert!(ema > 0.5, "EMA should trend towards 1.0, got {}", ema);
    }

    #[test]
    fn test_composite_detector_priority() {
        let invariants = Arc::new(InvariantLayer::new());
        let mut detector = CompositeDetector::standard(invariants);

        let inputs = HashMap::new();
        let outputs = HashMap::new();

        // With hard failure - should be detected first
        let error = EvalError::MissingInput(0);
        let result = detector.inspect(&inputs, &outputs, Some(&error));

        assert!(result.is_some());
        assert_eq!(
            result.unwrap().level,
            crate::evidence::AnomalyLevel::HardFailure
        );
    }
}
