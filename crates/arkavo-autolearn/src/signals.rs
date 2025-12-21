//! Pain signal aggregation
//!
//! Aggregates anomaly signals from multiple sources (runtime, boundary probing,
//! SAT analysis, Titan Monitor) and prioritizes them for synthesis.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use arkavo_ensemble::SynthesisContext;
use arkavo_sat::{AnomalyPrioritizer, BoundaryProbe, PolicyHole, ScoredNode};
use arkavo_titan::{AnomalyDetails, AnomalyEvidence, AnomalyLevel};
use torg_core::Graph;

/// Source of a pain signal
#[derive(Debug, Clone)]
pub enum PainSource {
    /// Runtime anomaly detected during evaluation
    RuntimeAnomaly {
        /// Output node that produced unexpected result
        output_id: u16,
        /// Hash of input that triggered the anomaly
        input_hash: u64,
        /// Expected output value
        expected: bool,
        /// Actual output value
        actual: bool,
    },
    /// Proactive boundary probe found vulnerability
    BoundaryProbe(BoundaryProbe),
    /// SAT solver found policy hole
    PolicyHole(PolicyHole),
    /// External signal from control plane
    External {
        /// Description of the external signal
        description: String,
    },
    /// Titan Monitor detected anomaly
    TitanAnomaly {
        /// Anomaly level (HardFailure, BoundaryViolation, StatisticalDrift)
        level: AnomalyLevel,
        /// Hash of input that triggered the anomaly
        input_hash: u64,
        /// Description of the anomaly
        description: String,
    },
}

impl PainSource {
    /// Get a human-readable description of the pain source
    #[must_use]
    pub fn description(&self) -> String {
        match self {
            Self::RuntimeAnomaly {
                output_id,
                expected,
                actual,
                ..
            } => {
                format!(
                    "Runtime anomaly at output {}: expected {}, got {}",
                    output_id, expected, actual
                )
            }
            Self::BoundaryProbe(probe) => {
                format!(
                    "Boundary probe: output {} at distance {}",
                    probe.output_id, probe.distance_to_flip
                )
            }
            Self::PolicyHole(hole) => {
                format!(
                    "Policy hole at output {}: {}",
                    hole.output_id, hole.description
                )
            }
            Self::External { description } => format!("External: {description}"),
            Self::TitanAnomaly { description, .. } => format!("Titan: {description}"),
        }
    }

    /// Get the output node ID if applicable
    #[must_use]
    pub fn output_id(&self) -> Option<u16> {
        match self {
            Self::RuntimeAnomaly { output_id, .. } => Some(*output_id),
            Self::BoundaryProbe(probe) => Some(probe.output_id),
            Self::PolicyHole(hole) => Some(hole.output_id),
            Self::External { .. } | Self::TitanAnomaly { .. } => None,
        }
    }
}

impl From<AnomalyEvidence> for PainSource {
    fn from(evidence: AnomalyEvidence) -> Self {
        let description = match &evidence.details {
            AnomalyDetails::EvalError { error } => format!("Evaluation error: {error}"),
            AnomalyDetails::BoundaryViolation {
                contract_id,
                description,
            } => format!(
                "Boundary violation (contract {}): {}",
                contract_id, description
            ),
            AnomalyDetails::Drift {
                output_id,
                baseline,
                current,
                z_score,
            } => format!(
                "Drift at output {}: baseline={:.3}, current={:.1}, z={:.2}",
                output_id, baseline, current, z_score
            ),
        };

        Self::TitanAnomaly {
            level: evidence.level,
            input_hash: evidence.input_hash,
            description,
        }
    }
}

/// Aggregated pain signal with context for synthesis
#[derive(Debug, Clone)]
pub struct PainSignal {
    /// Unique identifier for this signal
    pub id: Uuid,
    /// Source of the pain
    pub source: PainSource,
    /// Severity score (0.0-1.0, higher = more urgent)
    pub severity: f64,
    /// When the signal was detected
    pub timestamp: DateTime<Utc>,
    /// Pre-built synthesis context
    pub context: SynthesisContext,
}

impl PainSignal {
    /// Create a new pain signal
    #[must_use]
    pub fn new(source: PainSource, severity: f64, context: SynthesisContext) -> Self {
        Self {
            id: Uuid::new_v4(),
            source,
            severity: severity.clamp(0.0, 1.0),
            timestamp: Utc::now(),
            context,
        }
    }

    /// Create a pain signal from a boundary probe
    #[must_use]
    pub fn from_boundary_probe(probe: BoundaryProbe, graph: &Graph, model_id: String) -> Self {
        // Closer to boundary = higher severity
        let severity = 1.0 / (probe.distance_to_flip as f64 + 1.0);

        let context = SynthesisContext::new(
            model_id,
            format!(
                "Fix boundary vulnerability at output {} (flip distance: {})",
                probe.output_id, probe.distance_to_flip
            ),
        )
        .with_inputs(graph.inputs.clone())
        .with_outputs(graph.outputs.clone())
        .with_constraint(format!(
            "Current output {} = {}, should remain stable",
            probe.output_id, probe.output_value
        ));

        Self::new(PainSource::BoundaryProbe(probe), severity, context)
    }

    /// Create a pain signal from a policy hole
    #[must_use]
    pub fn from_policy_hole(hole: PolicyHole, graph: &Graph, model_id: String) -> Self {
        let severity = 0.9; // Policy holes are high priority

        let context = SynthesisContext::new(
            model_id,
            format!("Remediate policy hole: {}", hole.description),
        )
        .with_inputs(graph.inputs.clone())
        .with_outputs(graph.outputs.clone())
        .with_constraint(format!(
            "Must handle edge case at output {}",
            hole.output_id
        ));

        Self::new(PainSource::PolicyHole(hole), severity, context)
    }

    /// Create a pain signal from a runtime anomaly
    #[must_use]
    pub fn from_runtime_anomaly(
        output_id: u16,
        input_hash: u64,
        expected: bool,
        actual: bool,
        graph: &Graph,
        model_id: String,
    ) -> Self {
        let severity = 0.8; // Runtime anomalies are important

        let context = SynthesisContext::new(
            model_id,
            format!(
                "Fix runtime anomaly at output {}: expected {}, got {}",
                output_id, expected, actual
            ),
        )
        .with_inputs(graph.inputs.clone())
        .with_outputs(graph.outputs.clone())
        .with_constraint(format!(
            "Output {} must return {} for the failing case",
            output_id, expected
        ));

        Self::new(
            PainSource::RuntimeAnomaly {
                output_id,
                input_hash,
                expected,
                actual,
            },
            severity,
            context,
        )
    }
}

/// Default maximum signals to buffer
pub const DEFAULT_MAX_SIGNALS: usize = 100;

/// Aggregates pain signals from multiple sources
pub struct PainAggregator {
    /// Buffered signals waiting for processing
    signals: VecDeque<PainSignal>,
    /// Prioritizer for scoring nodes
    prioritizer: AnomalyPrioritizer,
    /// Maximum signals to buffer
    max_signals: usize,
}

impl PainAggregator {
    /// Create a new pain aggregator
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_SIGNALS)
    }

    /// Create with custom capacity
    #[must_use]
    pub fn with_capacity(max_signals: usize) -> Self {
        Self {
            signals: VecDeque::with_capacity(max_signals),
            prioritizer: AnomalyPrioritizer::new(),
            max_signals,
        }
    }

    /// Add a pain signal to the queue
    pub fn push(&mut self, signal: PainSignal) {
        // Track in prioritizer if we have an output ID
        if let Some(output_id) = signal.source.output_id() {
            self.prioritizer.record_violation(output_id);
        }

        // Add to queue
        self.signals.push_back(signal);

        // Evict oldest if over capacity
        while self.signals.len() > self.max_signals {
            self.signals.pop_front();
        }
    }

    /// Add signal from a boundary probe result
    pub fn from_probe_result(&mut self, probe: BoundaryProbe, graph: &Graph, model_id: String) {
        self.prioritizer.record_probe(probe.output_id, &probe);
        let signal = PainSignal::from_boundary_probe(probe, graph, model_id);
        self.push(signal);
    }

    /// Add signal from a policy hole
    pub fn from_policy_hole(&mut self, hole: PolicyHole, graph: &Graph, model_id: String) {
        let signal = PainSignal::from_policy_hole(hole, graph, model_id);
        self.push(signal);
    }

    /// Add signal from a runtime anomaly
    pub fn from_runtime_anomaly(
        &mut self,
        output_id: u16,
        input_hash: u64,
        expected: bool,
        actual: bool,
        graph: &Graph,
        model_id: String,
    ) {
        let signal = PainSignal::from_runtime_anomaly(
            output_id, input_hash, expected, actual, graph, model_id,
        );
        self.push(signal);
    }

    /// Add signal from Titan Monitor evidence
    ///
    /// Converts `AnomalyEvidence` from the Titan Monitor into a `PainSignal`
    /// and adds it to the queue for processing by the AutoLearner.
    pub fn from_titan_evidence(
        &mut self,
        evidence: AnomalyEvidence,
        graph: &Graph,
        model_id: String,
    ) {
        let severity = evidence.level.severity();
        let source = PainSource::from(evidence);

        let context = SynthesisContext::new(model_id, source.description())
            .with_inputs(graph.inputs.clone())
            .with_outputs(graph.outputs.clone());

        let signal = PainSignal::new(source, severity, context);
        self.push(signal);
    }

    /// Get the next highest-priority signal
    pub fn next_signal(&mut self) -> Option<PainSignal> {
        // Sort by severity (descending) and take the highest
        if self.signals.is_empty() {
            return None;
        }

        // Find highest severity index
        let max_idx = self
            .signals
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| {
                a.severity
                    .partial_cmp(&b.severity)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i);

        max_idx.and_then(|idx| self.signals.remove(idx))
    }

    /// Get number of pending signals
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.signals.len()
    }

    /// Check if there are any pending signals
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }

    /// Get the prioritizer for node scoring
    #[must_use]
    pub fn prioritizer(&self) -> &AnomalyPrioritizer {
        &self.prioritizer
    }

    /// Get mutable access to the prioritizer
    pub fn prioritizer_mut(&mut self) -> &mut AnomalyPrioritizer {
        &mut self.prioritizer
    }

    /// Select top nodes for proactive probing
    #[must_use]
    pub fn select_probe_targets(&self, graph: &Graph, k: usize) -> Vec<ScoredNode> {
        self.prioritizer.select_nodes(graph, k)
    }

    /// Clear all pending signals
    pub fn clear(&mut self) {
        self.signals.clear();
    }
}

impl Default for PainAggregator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
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
    fn test_pain_signal_from_boundary_probe() {
        let graph = create_simple_graph();
        let probe = BoundaryProbe {
            input_assignment: HashMap::new(),
            output_id: 1,
            output_value: true,
            distance_to_flip: 2,
        };

        let signal = PainSignal::from_boundary_probe(probe, &graph, "test-model".to_string());

        assert!(signal.severity > 0.0 && signal.severity <= 1.0);
        assert_eq!(signal.source.output_id(), Some(1));
        assert!(signal.source.description().contains("Boundary probe"));
    }

    #[test]
    fn test_pain_signal_from_runtime_anomaly() {
        let graph = create_simple_graph();
        let signal = PainSignal::from_runtime_anomaly(
            1,
            12345,
            true,
            false,
            &graph,
            "test-model".to_string(),
        );

        assert_eq!(signal.severity, 0.8);
        assert!(signal.source.description().contains("Runtime anomaly"));
    }

    #[test]
    fn test_pain_aggregator_ordering() {
        let mut agg = PainAggregator::new();

        let ctx = SynthesisContext::new("test".to_string(), "test".to_string());

        // Add signals with different severities
        agg.push(PainSignal::new(
            PainSource::External {
                description: "low".to_string(),
            },
            0.1,
            ctx.clone(),
        ));
        agg.push(PainSignal::new(
            PainSource::External {
                description: "high".to_string(),
            },
            0.9,
            ctx.clone(),
        ));
        agg.push(PainSignal::new(
            PainSource::External {
                description: "medium".to_string(),
            },
            0.5,
            ctx,
        ));

        // Should get highest severity first
        let first = agg.next_signal().unwrap();
        assert!((first.severity - 0.9).abs() < f64::EPSILON);

        let second = agg.next_signal().unwrap();
        assert!((second.severity - 0.5).abs() < f64::EPSILON);

        let third = agg.next_signal().unwrap();
        assert!((third.severity - 0.1).abs() < f64::EPSILON);

        assert!(agg.is_empty());
    }

    #[test]
    fn test_pain_aggregator_capacity() {
        let mut agg = PainAggregator::with_capacity(3);
        let ctx = SynthesisContext::new("test".to_string(), "test".to_string());

        // Add more than capacity
        for i in 0..5 {
            agg.push(PainSignal::new(
                PainSource::External {
                    description: format!("signal-{i}"),
                },
                i as f64 / 10.0,
                ctx.clone(),
            ));
        }

        // Should only have 3
        assert_eq!(agg.pending_count(), 3);
    }

    #[test]
    fn test_prioritizer_integration() {
        let graph = create_simple_graph();
        let mut agg = PainAggregator::new();

        // Record some probes
        let probe = BoundaryProbe {
            input_assignment: HashMap::new(),
            output_id: 1,
            output_value: true,
            distance_to_flip: 1,
        };
        agg.from_probe_result(probe, &graph, "model".to_string());

        // Should have recorded in prioritizer
        let history = agg.prioritizer().get_history(1);
        assert!(history.is_some());
        assert_eq!(history.unwrap().probe_count, 1);
    }
}
