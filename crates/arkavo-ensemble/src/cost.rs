//! Cost function trait and built-in implementations
//!
//! The cost function determines how "good" a policy decision is,
//! which is used for regret calculation.

use std::collections::HashMap;
use std::sync::Arc;

use torg_core::Graph;

/// Cost function trait for regret calculation
///
/// Implementations compute a cost value for a given input/output pair.
/// Lower costs indicate better decisions.
pub trait CostFunction: Send + Sync {
    /// Compute the cost for a given input/output pair
    ///
    /// # Arguments
    /// * `inputs` - The input values to the policy
    /// * `outputs` - The output values from the policy
    ///
    /// # Returns
    /// A cost value where lower is better
    fn compute(&self, inputs: &HashMap<u16, bool>, outputs: &HashMap<u16, bool>) -> f64;
}

/// Binary cost function: 0 if output matches expected, 1 otherwise
///
/// Useful when you have ground truth labels for what the policy should output.
#[derive(Debug, Clone)]
pub struct BinaryCost {
    /// Expected output values (keyed by output node ID)
    expected_outputs: HashMap<u16, bool>,
}

impl BinaryCost {
    /// Create a new binary cost function with expected outputs
    pub fn new(expected_outputs: HashMap<u16, bool>) -> Self {
        Self { expected_outputs }
    }

    /// Create from a single expected output
    pub fn single(output_id: u16, expected_value: bool) -> Self {
        let mut expected = HashMap::new();
        expected.insert(output_id, expected_value);
        Self::new(expected)
    }
}

impl CostFunction for BinaryCost {
    fn compute(&self, _inputs: &HashMap<u16, bool>, outputs: &HashMap<u16, bool>) -> f64 {
        for (id, expected) in &self.expected_outputs {
            if outputs.get(id) != Some(expected) {
                return 1.0;
            }
        }
        0.0
    }
}

/// Boundary distance cost function
///
/// Uses the distance to the decision boundary as the cost.
/// Policies that make decisions closer to the boundary have higher cost (riskier).
#[derive(Debug, Clone)]
pub struct BoundaryDistanceCost {
    /// The graph to compute boundary distance for
    graph: Arc<Graph>,
    /// The output node to measure distance for
    output_id: u16,
    /// Maximum epsilon to search for boundary
    max_epsilon: u32,
}

impl BoundaryDistanceCost {
    /// Create a new boundary distance cost function
    pub fn new(graph: Arc<Graph>, output_id: u16) -> Self {
        Self {
            graph,
            output_id,
            max_epsilon: 3, // Default max distance to search
        }
    }

    /// Create with custom max epsilon
    pub fn with_max_epsilon(graph: Arc<Graph>, output_id: u16, max_epsilon: u32) -> Self {
        Self {
            graph,
            output_id,
            max_epsilon,
        }
    }
}

impl CostFunction for BoundaryDistanceCost {
    fn compute(&self, inputs: &HashMap<u16, bool>, _outputs: &HashMap<u16, bool>) -> f64 {
        // Use arkavo-sat to compute flip distance
        use arkavo_sat::find_epsilon_boundary;

        // Find boundary probes
        match find_epsilon_boundary(&self.graph, self.output_id, self.max_epsilon) {
            Ok(probes) => {
                // Find the minimum flip distance for the given input
                let min_distance = probes
                    .iter()
                    .filter(|probe| {
                        // Check if this probe matches our input
                        probe
                            .input_assignment
                            .iter()
                            .all(|(id, val)| inputs.get(id) == Some(val))
                    })
                    .map(|probe| probe.distance_to_flip)
                    .min()
                    .unwrap_or(self.max_epsilon + 1);

                // Cost is inverse of distance (closer = higher cost)
                if min_distance == 0 {
                    1.0 // On the boundary
                } else {
                    1.0 / min_distance as f64
                }
            }
            Err(_) => {
                // If we can't compute boundary, return neutral cost
                0.5
            }
        }
    }
}

/// Constant cost function (for testing)
#[derive(Debug, Clone)]
pub struct ConstantCost {
    value: f64,
}

impl ConstantCost {
    /// Create a constant cost function
    pub fn new(value: f64) -> Self {
        Self { value }
    }

    /// Create a zero cost function
    pub fn zero() -> Self {
        Self::new(0.0)
    }
}

impl CostFunction for ConstantCost {
    fn compute(&self, _inputs: &HashMap<u16, bool>, _outputs: &HashMap<u16, bool>) -> f64 {
        self.value
    }
}

/// Weighted combination of multiple cost functions
pub struct WeightedCost {
    costs: Vec<(Box<dyn CostFunction>, f64)>,
}

impl WeightedCost {
    /// Create a new weighted cost function
    pub fn new() -> Self {
        Self { costs: Vec::new() }
    }

    /// Add a cost function with weight
    pub fn add<C: CostFunction + 'static>(mut self, cost: C, weight: f64) -> Self {
        self.costs.push((Box::new(cost), weight));
        self
    }
}

impl Default for WeightedCost {
    fn default() -> Self {
        Self::new()
    }
}

impl CostFunction for WeightedCost {
    fn compute(&self, inputs: &HashMap<u16, bool>, outputs: &HashMap<u16, bool>) -> f64 {
        let total_weight: f64 = self.costs.iter().map(|(_, w)| w).sum();
        if total_weight == 0.0 {
            return 0.0;
        }

        self.costs
            .iter()
            .map(|(c, w)| c.compute(inputs, outputs) * w)
            .sum::<f64>()
            / total_weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_cost_match() {
        let expected = BinaryCost::single(1, true);

        let mut outputs = HashMap::new();
        outputs.insert(1, true);

        let cost = expected.compute(&HashMap::new(), &outputs);
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_binary_cost_mismatch() {
        let expected = BinaryCost::single(1, true);

        let mut outputs = HashMap::new();
        outputs.insert(1, false);

        let cost = expected.compute(&HashMap::new(), &outputs);
        assert!((cost - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_binary_cost_multiple_outputs() {
        let mut expected_map = HashMap::new();
        expected_map.insert(1, true);
        expected_map.insert(2, false);
        let cost_fn = BinaryCost::new(expected_map);

        let mut outputs = HashMap::new();
        outputs.insert(1, true);
        outputs.insert(2, false);

        assert!((cost_fn.compute(&HashMap::new(), &outputs) - 0.0).abs() < f64::EPSILON);

        outputs.insert(2, true);
        assert!((cost_fn.compute(&HashMap::new(), &outputs) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_constant_cost() {
        let cost = ConstantCost::new(0.5);
        assert!((cost.compute(&HashMap::new(), &HashMap::new()) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_weighted_cost() {
        let weighted = WeightedCost::new()
            .add(ConstantCost::new(1.0), 1.0)
            .add(ConstantCost::new(0.0), 1.0);

        let cost = weighted.compute(&HashMap::new(), &HashMap::new());
        assert!((cost - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_weighted_cost_unequal_weights() {
        let weighted = WeightedCost::new()
            .add(ConstantCost::new(1.0), 3.0)
            .add(ConstantCost::new(0.0), 1.0);

        let cost = weighted.compute(&HashMap::new(), &HashMap::new());
        assert!((cost - 0.75).abs() < f64::EPSILON);
    }
}
