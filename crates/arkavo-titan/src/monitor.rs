//! TitanMonitor - Zero-copy evaluation wrapper with anomaly detection
//!
//! Wraps `torg_core::evaluate()` with automatic inspection for runtime anomalies.
//! Target overhead: <5µs per evaluation.

use std::collections::HashMap;
use std::sync::Arc;

use arkavo_sbe::InvariantLayer;
use tokio::sync::mpsc;
use torg_core::{EvalError, Graph};

use crate::detector::{CompositeDetector, SurpriseDetector};
use crate::evidence::AnomalyEvidence;

/// Default channel buffer size for evidence
pub const DEFAULT_CHANNEL_SIZE: usize = 256;

/// Configuration for Titan Monitor
#[derive(Debug, Clone)]
pub struct TitanConfig {
    /// Channel buffer size for evidence (drop if full)
    pub channel_size: usize,
    /// Enable drift detection (Level 3)
    pub enable_drift_detection: bool,
    /// EMA alpha for drift detection
    pub drift_alpha: f32,
    /// Z-score threshold for drift detection
    pub drift_z_threshold: f32,
    /// Minimum samples before drift detection activates
    pub drift_min_samples: u32,
}

impl Default for TitanConfig {
    fn default() -> Self {
        Self {
            channel_size: DEFAULT_CHANNEL_SIZE,
            enable_drift_detection: true,
            drift_alpha: 0.01,
            drift_z_threshold: 3.0,
            drift_min_samples: 100,
        }
    }
}

/// Titan Monitor - Zero-copy evaluation wrapper with anomaly detection
///
/// Wraps `torg_core::evaluate()` and inspects each transaction for anomalies.
/// Evidence is sent to a non-blocking channel (dropped if full).
///
/// # Performance
///
/// Target overhead: <5µs per evaluation (p99)
/// - torg_core::evaluate: ~0.4µs
/// - Hard failure check: ~0.1µs
/// - Boundary check: ~1.0µs
/// - Drift check: ~0.5µs
/// - Channel try_send: ~0.5µs
///
/// # Example
///
/// ```ignore
/// let invariants = Arc::new(InvariantLayer::new());
/// let (mut monitor, rx) = TitanMonitor::new(invariants, TitanConfig::default());
///
/// // Evaluate with monitoring
/// let result = monitor.evaluate(&graph, &inputs);
///
/// // Check for anomalies (non-blocking)
/// while let Ok(evidence) = rx.try_recv() {
///     println!("Anomaly: {:?}", evidence.level);
/// }
/// ```
pub struct TitanMonitor {
    /// Non-blocking sender for anomaly evidence
    evidence_tx: mpsc::Sender<AnomalyEvidence>,
    /// Composite detector chain
    detector: CompositeDetector,
}

impl TitanMonitor {
    /// Create a new Titan Monitor with the given invariants and configuration
    ///
    /// Returns the monitor and a receiver for anomaly evidence.
    #[must_use]
    pub fn new(
        invariants: Arc<InvariantLayer>,
        config: TitanConfig,
    ) -> (Self, mpsc::Receiver<AnomalyEvidence>) {
        let (tx, rx) = mpsc::channel(config.channel_size);

        let detector = if config.enable_drift_detection {
            let mut composite = CompositeDetector::new();
            composite.add(Box::new(crate::detector::HardFailureDetector::new()));
            composite.add(Box::new(crate::detector::BoundaryDetector::new(
                Arc::clone(&invariants),
            )));
            composite.add(Box::new(crate::detector::DriftDetector::with_config(
                config.drift_alpha,
                config.drift_z_threshold,
                config.drift_min_samples,
            )));
            composite
        } else {
            CompositeDetector::standard(invariants)
        };

        (Self { evidence_tx: tx, detector }, rx)
    }

    /// Create with custom detector chain
    #[must_use]
    pub fn with_detector(
        detector: CompositeDetector,
        channel_size: usize,
    ) -> (Self, mpsc::Receiver<AnomalyEvidence>) {
        let (tx, rx) = mpsc::channel(channel_size);
        (Self { evidence_tx: tx, detector }, rx)
    }

    /// Evaluate a graph with automatic anomaly detection
    ///
    /// # Performance
    ///
    /// Target overhead: <5µs over raw `torg_core::evaluate()`
    #[inline]
    pub fn evaluate(
        &mut self,
        graph: &Graph,
        inputs: &HashMap<u16, bool>,
    ) -> Result<HashMap<u16, bool>, EvalError> {
        // 1. Call underlying evaluate (~0.4µs)
        let result = torg_core::evaluate(graph, inputs);

        // 2. Inspect for anomalies (~2-3µs)
        let error = result.as_ref().err();
        let empty_outputs = HashMap::new();
        let outputs = result.as_ref().ok().unwrap_or(&empty_outputs);

        if let Some(evidence) = self.detector.inspect(inputs, outputs, error) {
            // 3. Non-blocking send (try_send, drop if full)
            let _ = self.evidence_tx.try_send(evidence);
        }

        result
    }

    /// Check if the evidence channel is still active
    #[must_use]
    pub fn is_connected(&self) -> bool {
        !self.evidence_tx.is_closed()
    }

    /// Get a reference to the detector for baseline updates
    pub fn detector_mut(&mut self) -> &mut CompositeDetector {
        &mut self.detector
    }
}

/// Standalone evaluation with monitoring (for one-off use)
///
/// Creates a temporary monitor, evaluates, and returns any evidence.
#[inline]
#[allow(dead_code)]
pub fn evaluate_with_inspection(
    graph: &Graph,
    inputs: &HashMap<u16, bool>,
    invariants: &InvariantLayer,
) -> (Result<HashMap<u16, bool>, EvalError>, Option<AnomalyEvidence>) {
    let result = torg_core::evaluate(graph, inputs);

    let error = result.as_ref().err();

    // Quick inspection without full detector chain
    let evidence = if let Some(e) = error {
        Some(AnomalyEvidence::hard_failure(
            e.clone(),
            crate::evidence::hash_inputs(inputs),
        ))
    } else if let Some(violation) = invariants.check_violation(inputs) {
        Some(AnomalyEvidence::boundary_violation(
            violation.contract_id,
            format!("Invariant violated at {:?}", violation.detected_at),
            crate::evidence::hash_inputs(inputs),
        ))
    } else {
        None
    };

    (result, evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{Builder, Token};

    fn create_simple_graph() -> Graph {
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn test_monitor_creation() {
        let invariants = Arc::new(InvariantLayer::new());
        let (monitor, _rx) = TitanMonitor::new(invariants, TitanConfig::default());
        assert!(monitor.is_connected());
    }

    #[test]
    fn test_successful_evaluation() {
        let invariants = Arc::new(InvariantLayer::new());
        let (mut monitor, _rx) = TitanMonitor::new(invariants, TitanConfig::default());

        let graph = create_simple_graph();
        let mut inputs = HashMap::new();
        inputs.insert(0, true);

        let result = monitor.evaluate(&graph, &inputs);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().get(&1), Some(&true));
    }

    #[test]
    fn test_error_detection() {
        let invariants = Arc::new(InvariantLayer::new());
        let (mut monitor, mut rx) = TitanMonitor::new(invariants, TitanConfig::default());

        let graph = create_simple_graph();
        let inputs = HashMap::new(); // Missing required input

        let result = monitor.evaluate(&graph, &inputs);
        assert!(result.is_err());

        // Should have evidence in channel
        let evidence = rx.try_recv();
        assert!(evidence.is_ok());
        assert_eq!(
            evidence.unwrap().level,
            crate::evidence::AnomalyLevel::HardFailure
        );
    }

    #[test]
    fn test_channel_drop_when_full() {
        let invariants = Arc::new(InvariantLayer::new());
        let config = TitanConfig {
            channel_size: 1,
            ..Default::default()
        };
        let (mut monitor, _rx) = TitanMonitor::new(invariants, config);

        let graph = create_simple_graph();
        let inputs = HashMap::new(); // Will trigger errors

        // Send multiple errors - channel should drop extras
        for _ in 0..10 {
            let _ = monitor.evaluate(&graph, &inputs);
        }

        // Should not panic or block
        assert!(monitor.is_connected());
    }

    #[test]
    fn test_standalone_evaluation() {
        let invariants = InvariantLayer::new();
        let graph = create_simple_graph();

        let mut inputs = HashMap::new();
        inputs.insert(0, true);

        let (result, evidence) = evaluate_with_inspection(&graph, &inputs, &invariants);

        assert!(result.is_ok());
        assert!(evidence.is_none()); // No anomaly
    }

    #[test]
    fn test_standalone_error_detection() {
        let invariants = InvariantLayer::new();
        let graph = create_simple_graph();
        let inputs = HashMap::new(); // Missing input

        let (result, evidence) = evaluate_with_inspection(&graph, &inputs, &invariants);

        assert!(result.is_err());
        assert!(evidence.is_some());
        assert_eq!(
            evidence.unwrap().level,
            crate::evidence::AnomalyLevel::HardFailure
        );
    }
}
