//! Anomaly prioritization for boundary probing
//!
//! Scores nodes based on historical violations and boundary proximity
//! to focus probing efforts on the most likely sources of policy holes.

use std::collections::HashMap;
use std::time::Instant;

use torg_core::Graph;

use crate::boundary::BoundaryProbe;

/// Default weights for priority scoring
pub const DEFAULT_VIOLATION_WEIGHT: f64 = 0.4;
pub const DEFAULT_RECENCY_WEIGHT: f64 = 0.2;
pub const DEFAULT_BOUNDARY_WEIGHT: f64 = 0.3;
pub const DEFAULT_EXPLORATION_WEIGHT: f64 = 0.1;

/// Decay constant for recency calculation (seconds)
pub const RECENCY_DECAY_SECONDS: f64 = 300.0;

/// Historical violation data for a node
#[derive(Debug, Clone, Default)]
pub struct NodeHistory {
    /// Total violation count
    pub violation_count: u32,
    /// Time of last violation
    pub last_violation: Option<Instant>,
    /// Number of times found near boundary
    pub boundary_hits: u32,
    /// Average distance to flip output
    pub avg_flip_distance: f32,
    /// Number of probes conducted
    pub probe_count: u32,
}

impl NodeHistory {
    /// Create a new empty history
    pub fn new() -> Self {
        Self::default()
    }

    /// Update with a new probe result
    pub fn update_from_probe(&mut self, probe: &BoundaryProbe) {
        self.probe_count += 1;
        self.boundary_hits += 1;

        // Update running average of flip distance
        let new_dist = probe.distance_to_flip as f32;
        if self.probe_count == 1 {
            self.avg_flip_distance = new_dist;
        } else {
            let n = self.probe_count as f32;
            self.avg_flip_distance = (self.avg_flip_distance * (n - 1.0) + new_dist) / n;
        }
    }

    /// Record a violation
    pub fn record_violation(&mut self) {
        self.violation_count += 1;
        self.last_violation = Some(Instant::now());
    }
}

/// Scoring weights for prioritization
#[derive(Debug, Clone)]
pub struct PriorityWeights {
    /// Weight for historical violations (higher = more violations = higher priority)
    pub violation_weight: f64,
    /// Weight for recent violations (higher = more recent = higher priority)
    pub recency_weight: f64,
    /// Weight for boundary proximity (higher = closer to boundary = higher priority)
    pub boundary_weight: f64,
    /// Weight for unexplored nodes (higher = less explored = higher priority)
    pub exploration_weight: f64,
}

impl Default for PriorityWeights {
    fn default() -> Self {
        Self {
            violation_weight: DEFAULT_VIOLATION_WEIGHT,
            recency_weight: DEFAULT_RECENCY_WEIGHT,
            boundary_weight: DEFAULT_BOUNDARY_WEIGHT,
            exploration_weight: DEFAULT_EXPLORATION_WEIGHT,
        }
    }
}

/// Reason why a node was prioritized
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorityReason {
    /// High historical violation rate
    HighViolationRate,
    /// Recent anomaly detected
    RecentAnomaly,
    /// Close to decision boundary
    NearBoundary,
    /// Not yet explored
    Unexplored,
}

/// Scored node for probing queue
#[derive(Debug, Clone)]
pub struct ScoredNode {
    /// Output node ID
    pub output_id: u16,
    /// Priority score (higher = higher priority)
    pub score: f64,
    /// Primary reason for priority
    pub reason: PriorityReason,
}

/// Prioritizer for selecting which nodes to probe
pub struct AnomalyPrioritizer {
    /// Historical data per node
    histories: HashMap<u16, NodeHistory>,
    /// Scoring weights
    weights: PriorityWeights,
    /// Maximum violation count seen (for normalization)
    max_violations: u32,
}

impl AnomalyPrioritizer {
    /// Create a new prioritizer with default weights
    pub fn new() -> Self {
        Self::with_weights(PriorityWeights::default())
    }

    /// Create a new prioritizer with custom weights
    pub fn with_weights(weights: PriorityWeights) -> Self {
        Self {
            histories: HashMap::new(),
            weights,
            max_violations: 1, // Avoid division by zero
        }
    }

    /// Calculate priority score for a node
    pub fn score_node(&self, output_id: u16) -> f64 {
        let history = self.histories.get(&output_id);

        match history {
            Some(h) => self.compute_score(h),
            None => {
                // Unexplored node gets exploration bonus
                self.weights.exploration_weight
            }
        }
    }

    /// Compute score from history
    fn compute_score(&self, history: &NodeHistory) -> f64 {
        let mut score = 0.0;

        // Violation component (normalized)
        let normalized_violations = history.violation_count as f64 / self.max_violations as f64;
        score += self.weights.violation_weight * normalized_violations;

        // Recency component
        if let Some(last) = history.last_violation {
            let elapsed = last.elapsed().as_secs_f64();
            let recency_factor = (-elapsed / RECENCY_DECAY_SECONDS).exp();
            score += self.weights.recency_weight * recency_factor;
        }

        // Boundary proximity component
        if history.avg_flip_distance > 0.0 {
            let boundary_factor = 1.0 / history.avg_flip_distance.max(1.0) as f64;
            score += self.weights.boundary_weight * boundary_factor;
        }

        // Exploration component (inverse of probe count)
        if history.probe_count == 0 {
            score += self.weights.exploration_weight;
        }

        score
    }

    /// Determine primary reason for a score
    fn determine_reason(&self, history: &NodeHistory) -> PriorityReason {
        if history.probe_count == 0 {
            return PriorityReason::Unexplored;
        }

        let mut max_component = 0.0;
        let mut reason = PriorityReason::NearBoundary;

        // Violation component
        let violation_score = self.weights.violation_weight
            * (history.violation_count as f64 / self.max_violations as f64);
        if violation_score > max_component {
            max_component = violation_score;
            reason = PriorityReason::HighViolationRate;
        }

        // Recency component
        if let Some(last) = history.last_violation {
            let elapsed = last.elapsed().as_secs_f64();
            let recency_score =
                self.weights.recency_weight * (-elapsed / RECENCY_DECAY_SECONDS).exp();
            if recency_score > max_component {
                max_component = recency_score;
                reason = PriorityReason::RecentAnomaly;
            }
        }

        // Boundary component
        if history.avg_flip_distance > 0.0 {
            let boundary_score =
                self.weights.boundary_weight * (1.0 / history.avg_flip_distance.max(1.0) as f64);
            if boundary_score > max_component {
                reason = PriorityReason::NearBoundary;
            }
        }

        reason
    }

    /// Get top-k nodes to probe next
    pub fn select_nodes(&self, graph: &Graph, k: usize) -> Vec<ScoredNode> {
        let mut scored: Vec<ScoredNode> = graph
            .outputs
            .iter()
            .map(|&output_id| {
                let score = self.score_node(output_id);
                let reason = self
                    .histories
                    .get(&output_id)
                    .map(|h| self.determine_reason(h))
                    .unwrap_or(PriorityReason::Unexplored);
                ScoredNode {
                    output_id,
                    score,
                    reason,
                }
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        scored.truncate(k);
        scored
    }

    /// Record a violation for a node
    pub fn record_violation(&mut self, output_id: u16) {
        let history = self.histories.entry(output_id).or_default();
        history.record_violation();

        // Update max for normalization
        if history.violation_count > self.max_violations {
            self.max_violations = history.violation_count;
        }
    }

    /// Record a boundary probe result
    pub fn record_probe(&mut self, output_id: u16, probe: &BoundaryProbe) {
        let history = self.histories.entry(output_id).or_default();
        history.update_from_probe(probe);
    }

    /// Reset history for a node (e.g., after graph modification)
    pub fn reset_node(&mut self, output_id: u16) {
        self.histories.remove(&output_id);
    }

    /// Reset all history
    pub fn reset_all(&mut self) {
        self.histories.clear();
        self.max_violations = 1;
    }

    /// Get history for a node
    pub fn get_history(&self, output_id: u16) -> Option<&NodeHistory> {
        self.histories.get(&output_id)
    }
}

impl Default for AnomalyPrioritizer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap as StdHashMap;
    use std::thread::sleep;
    use std::time::Duration;
    use torg_core::{Builder, Token};

    fn create_multi_output_graph() -> Graph {
        // Graph with 2 outputs
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();

        // Output 1: OR gate
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeEnd).unwrap();

        // Output 2: NOR gate
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(3)).unwrap();
        builder.push(Token::Nor).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeEnd).unwrap();

        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(3)).unwrap();

        builder.finish().unwrap()
    }

    #[test]
    fn test_unexplored_node_score() {
        let prioritizer = AnomalyPrioritizer::new();

        // Unknown node should get exploration bonus
        let score = prioritizer.score_node(999);
        assert!((score - DEFAULT_EXPLORATION_WEIGHT).abs() < 0.001);
    }

    #[test]
    fn test_violation_increases_score() {
        let mut prioritizer = AnomalyPrioritizer::new();

        let initial_score = prioritizer.score_node(2);

        prioritizer.record_violation(2);
        let after_violation = prioritizer.score_node(2);

        assert!(after_violation > initial_score);
    }

    #[test]
    fn test_probe_updates_history() {
        let mut prioritizer = AnomalyPrioritizer::new();

        let probe = BoundaryProbe {
            input_assignment: StdHashMap::new(),
            output_id: 2,
            output_value: true,
            distance_to_flip: 2,
        };

        prioritizer.record_probe(2, &probe);

        let history = prioritizer.get_history(2).unwrap();
        assert_eq!(history.probe_count, 1);
        assert_eq!(history.boundary_hits, 1);
        assert!((history.avg_flip_distance - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_select_nodes_ordering() {
        let graph = create_multi_output_graph();
        let mut prioritizer = AnomalyPrioritizer::new();

        // Add violations to node 2
        prioritizer.record_violation(2);
        prioritizer.record_violation(2);

        let selected = prioritizer.select_nodes(&graph, 2);
        assert_eq!(selected.len(), 2);

        // Node 2 should have higher score due to violations
        assert_eq!(selected[0].output_id, 2);
    }

    #[test]
    fn test_recency_decay() {
        let mut prioritizer = AnomalyPrioritizer::new();

        prioritizer.record_violation(2);
        let immediate_score = prioritizer.score_node(2);

        // Wait a bit
        sleep(Duration::from_millis(50));
        let later_score = prioritizer.score_node(2);

        // Score should decrease over time (recency decay)
        assert!(later_score < immediate_score);
    }

    #[test]
    fn test_boundary_proximity_score() {
        let mut prioritizer = AnomalyPrioritizer::new();

        // Node close to boundary
        let close_probe = BoundaryProbe {
            input_assignment: StdHashMap::new(),
            output_id: 2,
            output_value: true,
            distance_to_flip: 1,
        };
        prioritizer.record_probe(2, &close_probe);

        // Node far from boundary
        let far_probe = BoundaryProbe {
            input_assignment: StdHashMap::new(),
            output_id: 3,
            output_value: true,
            distance_to_flip: 5,
        };
        prioritizer.record_probe(3, &far_probe);

        let close_score = prioritizer.score_node(2);
        let far_score = prioritizer.score_node(3);

        // Closer to boundary should have higher score
        assert!(close_score > far_score);
    }

    #[test]
    fn test_reset_node() {
        let mut prioritizer = AnomalyPrioritizer::new();

        prioritizer.record_violation(2);
        assert!(prioritizer.get_history(2).is_some());

        prioritizer.reset_node(2);
        assert!(prioritizer.get_history(2).is_none());
    }

    #[test]
    fn test_priority_reason() {
        let graph = create_multi_output_graph();
        let mut prioritizer = AnomalyPrioritizer::new();

        // Unexplored node
        let selected = prioritizer.select_nodes(&graph, 1);
        assert_eq!(selected[0].reason, PriorityReason::Unexplored);

        // Node with violation - after recording, history exists but probe_count is 0
        // so it will still be Unexplored until we also probe it
        prioritizer.record_violation(2);

        // Add a probe to make it not unexplored
        let probe = BoundaryProbe {
            input_assignment: StdHashMap::new(),
            output_id: 2,
            output_value: true,
            distance_to_flip: 1,
        };
        prioritizer.record_probe(2, &probe);

        let selected = prioritizer.select_nodes(&graph, 2);
        let node2 = selected.iter().find(|n| n.output_id == 2).unwrap();
        // Now should be either HighViolationRate, RecentAnomaly, or NearBoundary
        assert!(matches!(
            node2.reason,
            PriorityReason::HighViolationRate
                | PriorityReason::RecentAnomaly
                | PriorityReason::NearBoundary
        ));
    }

    #[test]
    fn test_custom_weights() {
        let weights = PriorityWeights {
            violation_weight: 1.0,
            recency_weight: 0.0,
            boundary_weight: 0.0,
            exploration_weight: 0.0,
        };
        let mut prioritizer = AnomalyPrioritizer::with_weights(weights);

        prioritizer.record_violation(2);

        // With only violation weight, score should equal normalized violations
        let score = prioritizer.score_node(2);
        assert!((score - 1.0).abs() < 0.001); // 1 violation / 1 max = 1.0
    }
}
